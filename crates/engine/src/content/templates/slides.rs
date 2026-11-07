//! Presentation slides: five 1920 × 1080 slides of an invented season review (title, agenda, a big
//! figure, a chart, closing) with their own text styles and a footer component whose page number
//! is a property.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::glyphs;
use super::kit::{ACCENT, BORDER, BOTTOM_EDGE, CARD, DEEP, H, INK, Kit, MUTED, ON_BRAND, PAGE, PRIMARY, TOP_EDGE, V, merged};
use crate::Result;

const W: f64 = 1920.0;
const SLIDE_H: f64 = 1080.0;
const GAP: f64 = 160.0;

const AGENDA: [&str; 4] = ["The season in numbers", "What walkers asked for", "What we are building next", "Questions"];
const WEEKS: [(&str, f64); 10] =
    [("W1", 0.30), ("W2", 0.36), ("W3", 0.44), ("W4", 0.41), ("W5", 0.55), ("W6", 0.63), ("W7", 0.72), ("W8", 0.95), ("W9", 0.84), ("W10", 0.77)];

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.type_styles(&[
        ("Slide/Figure", 260.0, "Bold", 260.0),
        ("Slide/Title", 120.0, "Bold", 128.0),
        ("Slide/Heading", 72.0, "Bold", 84.0),
        ("Slide/Body", 40.0, "Regular", 58.0),
        ("Slide/Label", 26.0, "Medium", 36.0),
    ])?;
    k.rename_page("Slides")?;
    let page = k.page()?;
    let parts = k.add_page("Components")?;
    let footer = footer(k, parts)?;
    title(k, page)?;
    agenda(k, page, footer)?;
    figure(k, page, footer)?;
    chart(k, page, footer)?;
    closing(k, page)
}

/// The footer of the inner slides, with the page number as a text property.
fn footer(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let props = json!({"sizingH": "fixed", "width": W - 240.0, "mainAlign": "spaceBetween", "crossAlign": "center", "stroke": BORDER, "strokeSides": TOP_EDGE});
    let c = k.stack_of("component", page, "Footer", H, 16.0, json!([28, 0, 0, 0]), props)?;
    let logo = k.stack(c, "Logo", H, 12.0, json!(0), json!({"crossAlign": "center"}))?;
    k.icon(logo, &glyphs::MOUNTAIN, 32.0, PRIMARY)?;
    k.text(logo, "Ridgeline Studio · Season review 2026", "Slide/Label", MUTED)?;
    let number = k.text(c, "1", "Slide/Label", MUTED)?;
    k.text_prop(c, "Page", "1", &[number])?;
    Ok(c)
}

/// Slide `index` (from 0): a 1920 × 1080 frame in a three-column grid on the canvas. Its content
/// goes in the returned frame; inner slides get the footer under it.
fn slide(k: &mut Kit, page: NodeId, name: &str, index: usize, dark: bool, footer: Option<NodeId>) -> Result<NodeId> {
    let (x, y) = ((index % 3) as f64 * (W + GAP), (index / 3) as f64 * (SLIDE_H + GAP));
    let fill = if dark { DEEP } else { CARD };
    let frame = json!({"fill": fill, "stroke": BORDER, "strokeWeight": 1, "strokeAlign": "outside", "clip": true, "sizingH": "fixed", "sizingV": "fixed", "width": W, "height": SLIDE_H, "x": x, "y": y});
    let root = k.stack(page, name, V, 40.0, json!([120, 120, 72, 120]), frame)?;
    let content = k.stack(root, "Content", V, 48.0, json!(0), json!({"sizingH": "fill", "sizingV": "fill", "mainAlign": "center"}))?;
    if let Some(footer) = footer {
        k.instance(footer, root, json!({"Page": (index + 1).to_string()}), json!({"sizingH": "fill"}))?;
    }
    Ok(content)
}

fn wrap() -> Value {
    json!({"sizingH": "fill", "textAutoResize": "height"})
}

fn title(k: &mut Kit, page: NodeId) -> Result<()> {
    let content = slide(k, page, "1 Title", 0, true, None)?;
    k.set(content, json!({"layout": H, "gap": 96, "crossAlign": "center"}))?;
    let words = k.stack(content, "Words", V, 32.0, json!(0), json!({"sizingH": "fill"}))?;
    k.text(words, "SEASON REVIEW · 2026", "Slide/Label", ACCENT)?;
    k.text_with(words, "A summer on the ridge", "Slide/Title", ON_BRAND, wrap())?;
    k.text_with(words, "What 4,200 clubs walked, and what we learned from them", "Slide/Body", ON_BRAND, merged(wrap(), json!({"opacity": 0.8})))?;
    k.scene(content, "Picture", [640.0, 720.0], json!({"cornerRadius": 48})).map(drop)
}

fn agenda(k: &mut Kit, page: NodeId, footer: NodeId) -> Result<()> {
    let content = slide(k, page, "2 Agenda", 1, false, Some(footer))?;
    k.text(content, "Today", "Slide/Heading", INK)?;
    let list = k.stack(content, "List", V, 0.0, json!(0), json!({"sizingH": "fill"}))?;
    for (i, item) in AGENDA.into_iter().enumerate() {
        let row = json!({"sizingH": "fill", "crossAlign": "center", "stroke": BORDER, "strokeSides": BOTTOM_EDGE});
        let row = k.stack(list, item, H, 48.0, json!([28, 0, 28, 0]), row)?;
        k.text(row, &format!("{:02}", i + 1), "Slide/Body", PRIMARY)?;
        k.text_with(row, item, "Slide/Body", INK, json!({"sizingH": "fill"}))?;
    }
    Ok(())
}

fn figure(k: &mut Kit, page: NodeId, footer: NodeId) -> Result<()> {
    let content = slide(k, page, "3 Figure", 2, false, Some(footer))?;
    k.set(content, json!({"gap": 8}))?;
    k.text(content, "38,000", "Slide/Figure", PRIMARY)?;
    k.text_with(
        content,
        "routes shared between June and September: three times as many as last summer",
        "Slide/Body",
        MUTED,
        merged(wrap(), json!({"maxWidth": 1100})),
    )
    .map(drop)
}

fn chart(k: &mut Kit, page: NodeId, footer: NodeId) -> Result<()> {
    let content = slide(k, page, "4 Chart", 3, false, Some(footer))?;
    k.set(content, json!({"layout": H, "gap": 96, "crossAlign": "center"}))?;
    let words = k.stack(content, "Words", V, 32.0, json!(0), json!({"sizingH": "fixed", "width": 620}))?;
    k.text_with(words, "Walks peaked in week eight", "Slide/Heading", INK, wrap())?;
    k.text_with(words, "The long weekend filled every guided walk. Weeks nine and ten held most of it.", "Slide/Body", MUTED, wrap())?;
    let card = k.stack(
        content,
        "Walks per week",
        V,
        24.0,
        json!(48),
        json!({"fill": PAGE, "cornerRadius": 40, "sizingH": "fill", "sizingV": "fixed", "height": 640}),
    )?;
    k.text(card, "Walks per week", "Slide/Label", INK)?;
    k.bar_chart(card, [900.0, 440.0], &WEEKS, "Slide/Label")
}

fn closing(k: &mut Kit, page: NodeId) -> Result<()> {
    let content = slide(k, page, "5 Closing", 4, true, None)?;
    k.set(content, json!({"gap": 24}))?;
    k.icon(content, &glyphs::MOUNTAIN, 96.0, ACCENT)?;
    k.text(content, "Thank you", "Slide/Title", ON_BRAND)?;
    k.text_with(content, "Questions and routes to hello@ridgeline.example", "Slide/Body", ON_BRAND, json!({"opacity": 0.8})).map(drop)
}
