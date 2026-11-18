#[cfg(test)]
use super::axes::round_axis;
#[cfg(test)]
use super::picker::{FontPick, font_picker};
#[cfg(test)]
use super::spacing::{parse_letter_spacing, parse_line_height};
use super::{Align2, Arc, Color32, CornerRadius, FIELD_H, Icon, Id, Rect, Sense, Stroke, StrokeKind, Tokens, Ui, icons, pos2, regular, tr, vec2};

pub(super) fn caption(ui: &mut Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(egui::RichText::new(s).font(regular(10.5)).color(t.text_secondary));
}

/// A segmented control with short text labels (decoration, case, list…); `tip`s are English keys
/// (matched below), translated where shown. Returns the clicked index.
pub(super) fn text_seg(ui: &mut Ui, salt: &str, items: &[(&str, &str)], active: Option<usize>, width: f32) -> Option<usize> {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(width, FIELD_H), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(5), t.field);
    let cell = width / items.len().max(1) as f32;
    let mut out = None;
    for (i, (label, tip)) in items.iter().enumerate() {
        let r = Rect::from_min_size(pos2(rect.min.x + i as f32 * cell, rect.min.y), vec2(cell, FIELD_H));
        let resp = ui.interact(r, Id::unique(("ty_seg", salt, i)), Sense::click()).on_hover_text(tr(tip));
        crate::a11y::toggle(&resp, tr(tip), active == Some(i));
        if active == Some(i) {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.panel);
            ui.painter().rect_stroke(r.shrink(2.0), CornerRadius::same(4), Stroke::new(1.0, t.border), StrokeKind::Inside);
        } else if resp.hovered() {
            ui.painter().rect_filled(r.shrink(2.0), CornerRadius::same(4), t.hover);
        }
        let col = if active == Some(i) { t.text } else { t.text_secondary };
        let galley = if *tip == "Small caps" {
            small_caps_sample(ui, label, col)
        } else {
            ui.painter().layout_no_wrap((*label).to_string(), regular(11.5), col)
        };
        let gr = Rect::from_center_size(r.center(), galley.size());
        let size = galley.size();
        ui.painter().galley(gr.min, galley, col);
        // Draw the decoration on its own sample letter.
        match *tip {
            "Underline" => {
                let y = gr.max.y - 1.5;
                ui.painter().line_segment([pos2(gr.min.x, y), pos2(gr.min.x + size.x, y)], Stroke::new(1.0, col));
            }
            "Strikethrough" => {
                let y = gr.center().y + 0.5;
                ui.painter().line_segment([pos2(gr.min.x - 1.0, y), pos2(gr.min.x + size.x + 1.0, y)], Stroke::new(1.0, col));
            }
            _ => {}
        }
        if resp.clicked() {
            out = Some(i);
        }
    }
    out
}

/// The Small caps sample: a capital, then its other letters as smaller capitals. Drawn from
/// ordinary capitals because the small-capital letters (`ɢ`, U+0262) are missing from many UI
/// fonts, egui's defaults among them, where they showed as a box.
fn small_caps_sample(ui: &Ui, label: &str, col: Color32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    let mut chars = label.chars();
    let first: String = chars.next().into_iter().collect();
    let rest: String = chars.map(small_capital_to_capital).collect::<String>().to_uppercase();
    job.append(&first, 0.0, egui::TextFormat { font_id: regular(11.5), color: col, valign: egui::Align::BOTTOM, ..Default::default() });
    job.append(&rest, 0.0, egui::TextFormat { font_id: regular(8.5), color: col, valign: egui::Align::BOTTOM, ..Default::default() });
    ui.painter().layout_job(job)
}

/// The capital a small-capital letter stands for (the ones a sample uses), else the letter.
fn small_capital_to_capital(c: char) -> char {
    match c {
        'ɢ' => 'G',
        'ʜ' => 'H',
        'ɪ' => 'I',
        'ʟ' => 'L',
        'ɴ' => 'N',
        'ʀ' => 'R',
        'ʏ' => 'Y',
        c => c,
    }
}

/// One OpenType feature toggle; `on` None = mixed. Returns true when clicked.
pub(super) fn feature_row(ui: &mut Ui, tag: &str, label: &str, on: Option<bool>, supported: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
    let resp = if supported { resp } else { resp.on_hover_text(tr("Not in this font")) };
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), t.hover);
    }
    let b = Rect::from_min_size(pos2(rect.min.x + 4.0, rect.center().y - 7.0), vec2(14.0, 14.0));
    match on {
        Some(true) => {
            ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
            icons::paint(ui.painter(), b.shrink(1.5), Icon::Check, t.accent_on);
        }
        Some(false) => {
            ui.painter().rect_stroke(b, CornerRadius::same(3), Stroke::new(1.0, t.border_strong), StrokeKind::Inside);
        }
        None => {
            ui.painter().rect_filled(b, CornerRadius::same(3), t.accent);
            ui.painter().line_segment([pos2(b.min.x + 3.5, b.center().y), pos2(b.max.x - 3.5, b.center().y)], Stroke::new(1.5, t.accent_on));
        }
    }
    let col = if supported { t.text } else { t.text_tertiary };
    ui.painter().text(pos2(b.max.x + 8.0, rect.center().y), Align2::LEFT_CENTER, label, regular(11.0), col);
    ui.painter().text(pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, tag, regular(10.0), t.text_tertiary);
    resp.clicked()
}

#[derive(Clone, Copy)]
pub(super) enum LeadGlyph {
    LineHeight,
    LetterSpacing,
    ParagraphSpacing,
}

/// Paint a small original glyph for a typography field's lead.
pub(super) fn paint_lead(p: &egui::Painter, r: Rect, g: LeadGlyph, col: Color32) {
    let c = r.center();
    let s = Stroke::new(1.0, col);
    match g {
        LeadGlyph::LineHeight => {
            p.line_segment([pos2(c.x - 5.0, c.y - 6.0), pos2(c.x + 5.0, c.y - 6.0)], s);
            p.line_segment([pos2(c.x - 5.0, c.y + 6.0), pos2(c.x + 5.0, c.y + 6.0)], s);
            p.text(c, Align2::CENTER_CENTER, "A", regular(9.5), col);
        }
        LeadGlyph::LetterSpacing => {
            p.line_segment([pos2(c.x - 6.0, c.y - 5.0), pos2(c.x - 6.0, c.y + 5.0)], s);
            p.line_segment([pos2(c.x + 6.0, c.y - 5.0), pos2(c.x + 6.0, c.y + 5.0)], s);
            p.text(c, Align2::CENTER_CENTER, "A", regular(9.5), col);
        }
        LeadGlyph::ParagraphSpacing => {
            for dy in [-5.0, -2.5, 2.5, 5.0] {
                p.line_segment([pos2(c.x - 5.0, c.y + dy), pos2(c.x + 5.0, c.y + dy)], s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::typography::{target_styles, uniform};
    use figcraft_doc::{Document, LetterSpacing, LineHeight, Node, NodeKind, TextData, TextStyle};
    use serde_json::json;

    #[test]
    fn small_caps_sample_draws_only_letters_the_ui_font_has() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx, false, 0);
        let mut shown = None;
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            shown = Some(small_caps_sample(ui, "Aɢ", Color32::WHITE));
        });
        out.textures_delta.clear();
        let g = shown.expect("drawn");
        assert_eq!(g.job.text, "AG");
        let [a, rest] = g.job.sections.as_slice() else { panic!("two sections: {:?}", g.job.sections) };
        assert!(rest.format.font_id.size < a.format.font_id.size, "the other letters are smaller capitals");
        ctx.fonts_mut(|f| {
            for sec in &g.job.sections {
                assert!(f.has_glyphs(&sec.format.font_id, &g.job.text), "UI font draws {:?}", g.job.text);
            }
        });
        assert_eq!(small_capital_to_capital('ʀ'), 'R');
        assert_eq!(small_capital_to_capital('x'), 'x');
    }

    #[test]
    fn parses_line_height_and_letter_spacing() {
        assert_eq!(parse_line_height("Auto", None), Some(json!("auto")));
        assert_eq!(parse_line_height("", Some(LineHeight::Pixels(20.0))), Some(json!("auto")));
        assert_eq!(parse_line_height("150%", Some(LineHeight::Auto)), Some(json!("150%")));
        assert_eq!(parse_line_height("24", Some(LineHeight::Auto)), Some(json!(24.0)));
        assert_eq!(parse_line_height("24px", Some(LineHeight::Percent(120.0))), Some(json!(24.0)));
        assert_eq!(parse_line_height("130", Some(LineHeight::Percent(120.0))), Some(json!("130%")));
        assert_eq!(parse_line_height("-5", Some(LineHeight::Pixels(10.0))), Some(json!(0.0)));
        assert_eq!(parse_line_height("abc", None), None);
        // Enter on an untouched Mixed field commits nothing; typing Auto still does.
        assert_eq!(parse_line_height("", None), None);
        assert_eq!(parse_line_height("  ", None), None);
        assert_eq!(parse_letter_spacing("2%", None), Some(json!("2%")));
        assert_eq!(parse_letter_spacing("1.5px", None), Some(json!(1.5)));
        assert_eq!(parse_letter_spacing("-1", None), Some(json!("-1%")));
        assert_eq!(parse_letter_spacing("3", Some(LetterSpacing::Pixels(1.0))), Some(json!(3.0)));
        assert_eq!(parse_letter_spacing("x", Some(LetterSpacing::Percent(0.0))), None);
    }

    /// One frame of the font picker under a field, with `events`. `opened` clicks the field first
    /// (a press frame, then the release frame this returns).
    fn pick(ctx: &egui::Context, current: &str, opened: bool, events: Vec<egui::Event>) -> FontPick {
        let field = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(200.0, 24.0));
        let frame = |events: Vec<egui::Event>| {
            let mut out = FontPick::default();
            let mut shown = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                let resp = ui.interact(field, egui::Id::unique("font_field"), egui::Sense::click());
                out = font_picker(ui, &resp, Some(current), 240.0, |_| false).unwrap_or_default();
            });
            shown.textures_delta.clear();
            out
        };
        if opened {
            let button = |pressed| egui::Event::PointerButton {
                pos: field.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            // The field is laid out once before the pointer can hit it.
            frame(vec![]);
            frame(vec![egui::Event::PointerMoved(field.center())]);
            frame(vec![button(true)]);
            return frame(vec![button(false)]);
        }
        frame(events)
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn arrow_keys_step_through_the_families_and_enter_picks() {
        // #345
        figcraft_text::scan_system_fonts();
        let all = figcraft_text::family_names();
        if all.len() < 3 {
            eprintln!("fewer than three font families; skipping");
            return;
        }
        let ctx = egui::Context::default();
        let current = all[1].as_str();
        assert!(pick(&ctx, current, true, vec![]).highlighted.is_none());
        // The first step starts from the current family; each press is one step.
        let down = pick(&ctx, current, false, vec![key(egui::Key::ArrowDown)]);
        assert_eq!(down.highlighted.as_deref(), Some(all[2].as_str()));
        let up = pick(&ctx, current, false, vec![key(egui::Key::ArrowUp), key(egui::Key::ArrowUp)]);
        assert_eq!(up.highlighted.as_deref(), Some(all[0].as_str()));
        // At the top it stays; Enter picks the family it is on, not the first.
        assert_eq!(pick(&ctx, current, false, vec![key(egui::Key::ArrowUp)]).highlighted.as_deref(), Some(all[0].as_str()));
        pick(&ctx, current, false, vec![key(egui::Key::ArrowDown)]);
        let enter = pick(&ctx, current, false, vec![key(egui::Key::Enter)]);
        assert_eq!(enter.picked.as_deref(), Some(all[1].as_str()));
    }

    #[test]
    fn axis_values_round_by_axis_width() {
        let axis = |min: f64, max: f64| figcraft_text::Axis { tag: "test".into(), name: "Test".into(), min, default: min, max, hidden: false };
        assert_eq!(round_axis(&axis(100.0, 900.0), 456.6), 457.0);
        assert_eq!(round_axis(&axis(-10.0, 0.0), -4.26), -4.3);
        assert_eq!(round_axis(&axis(0.0, 1.0), 0.04), 0.0);
    }

    #[test]
    fn uniform_reports_mixed() {
        let a = TextStyle::default();
        let b = TextStyle { font_size: 30.0, ..Default::default() };
        assert_eq!(uniform(&[a.clone(), b.clone()], |s| s.font_family.clone()), Some("Inter".to_string()));
        assert_eq!(uniform(&[a.clone(), b], |s| s.font_size), None);
        assert_eq!(uniform(&[], |s: &TextStyle| s.font_size), None);
        // Ranges pick only the spans they touch.
        let mut doc = Document::new("t");
        let page = doc.first_page().unwrap();
        let id = doc.alloc_id();
        let mut td = TextData::new("Hello world");
        td.restyle(6, 11, |s| s.font_size = 40.0);
        doc.insert(Node::new(id, NodeKind::Text(td), "t", 10.0, 10.0), page, None).unwrap();
        assert_eq!(target_styles(&doc, &[id], Some((id, 0, 5))).len(), 1);
        assert_eq!(target_styles(&doc, &[id], Some((id, 4, 8))).len(), 2);
        assert_eq!(target_styles(&doc, &[id], Some((id, 3, 3))).len(), 2);
        assert_eq!(target_styles(&doc, &[id], None).len(), 2);
    }
}
