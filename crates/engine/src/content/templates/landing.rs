//! Landing page: the page of an invented product ("Ridgeline Studio") as one auto layout frame per
//! width. Sections fill the frame and hold a container with a maximum width, so the desktop frame
//! can be resized; the phone frame is the same page built with its rows stacked.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::glyphs::{self, Glyph};
use super::kit::{BORDER, BOTTOM_EDGE, CARD, H, ICON, INK, Kit, MUTED, ON_BRAND, PAGE, PRIMARY, SOFT, SUNKEN, TOP_EDGE, V, merged, shadow};
use crate::Result;

/// The widest the content gets.
const CONTAINER: f64 = 1120.0;

const FEATURES: [(&str, Glyph, &str); 3] = [
    ("Routes everyone can edit", glyphs::LAYERS, "Draw a route once and let the whole club adjust it. Every change is kept, so nothing is lost."),
    ("Ready without signal", glyphs::DOWNLOAD, "Maps, notes and meeting points are saved to each phone before you leave the car park."),
    ("Plans that fit the day", glyphs::CLOCK, "Sunrise, daylight and pace are worked out for you, with a turnaround time you can trust."),
];

const FIGURES: [(&str, &str); 4] =
    [("4,200", "Clubs planning together"), ("38k", "Routes shared"), ("96%", "Walks finished on time"), ("12", "Languages")];

/// One width of the page.
struct Layout {
    width: f64,
    /// Rows become columns.
    narrow: bool,
}

impl Layout {
    fn row(&self) -> &'static str {
        if self.narrow { V } else { H }
    }

    /// Room above and below a section.
    fn pad(&self, wide: f64) -> f64 {
        if self.narrow { wide / 2.0 } else { wide }
    }

    /// How a row that became a column lines its content up.
    fn cross(&self) -> &'static str {
        if self.narrow { "min" } else { "center" }
    }
}

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.type_styles(&[("Display/XL", 64.0, "Bold", 72.0)])?;
    k.rename_page("Landing page")?;
    let page = k.page()?;
    page_at(k, page, "Desktop", 0.0, &Layout { width: 1440.0, narrow: false })?;
    page_at(k, page, "Phone", 1540.0, &Layout { width: 390.0, narrow: true })
}

fn page_at(k: &mut Kit, page: NodeId, name: &str, x: f64, l: &Layout) -> Result<()> {
    let props = json!({"fill": CARD, "stroke": BORDER, "strokeWeight": 1, "strokeAlign": "outside", "clip": true, "sizingH": "fixed", "width": l.width, "x": x});
    let root = k.stack(page, name, V, 0.0, json!(0), props)?;
    nav(k, root, l)?;
    hero(k, root, l)?;
    features(k, root, l)?;
    figures(k, root, l)?;
    quote(k, root, l)?;
    call_to_action(k, root, l)?;
    footer(k, root, l)
}

/// A full-width band (with room above and below) holding a centred container that lays its
/// content out in `dir`, `gap` apart.
fn section(k: &mut Kit, root: NodeId, name: &str, l: &Layout, dir: &str, [top, bottom, gap]: [f64; 3], props: Value) -> Result<NodeId> {
    let side = if l.narrow { 20.0 } else { 40.0 };
    let band = merged(json!({"sizingH": "fill", "crossAlign": "center"}), props);
    let band = k.stack(root, name, V, 0.0, json!([l.pad(top), side, l.pad(bottom), side]), band)?;
    k.stack(band, "Container", dir, gap, json!(0), json!({"sizingH": "fill", "maxWidth": CONTAINER}))
}

fn logo(k: &mut Kit, parent: NodeId) -> Result<NodeId> {
    let logo = k.stack(parent, "Logo", H, 8.0, json!(0), json!({"crossAlign": "center"}))?;
    k.icon(logo, &glyphs::MOUNTAIN, ICON, PRIMARY)?;
    k.text(logo, "Ridgeline Studio", "Heading/S", INK)?;
    Ok(logo)
}

fn button(k: &mut Kit, parent: NodeId, label: &str, primary: bool) -> Result<NodeId> {
    let (color, fill) = if primary { (ON_BRAND, PRIMARY) } else { (INK, SUNKEN) };
    k.chip(parent, label, ["Label/M", color], fill, [14.0, 22.0], 14.0)
}

fn nav(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let bar = section(k, root, "Navigation", l, H, [20.0, 20.0, 24.0], json!({"stroke": BORDER, "strokeSides": BOTTOM_EDGE}))?;
    k.set(bar, json!({"mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    logo(k, bar)?;
    if l.narrow {
        k.icon(bar, &glyphs::MENU, ICON, INK)?;
        return Ok(());
    }
    let links = k.stack(bar, "Links", H, 36.0, json!(0), json!({}))?;
    for link in ["Product", "Pricing", "Stories", "Help"] {
        k.text(links, link, "Label/M", MUTED)?;
    }
    let actions = k.stack(bar, "Actions", H, 20.0, json!(0), json!({"crossAlign": "center"}))?;
    k.text(actions, "Sign in", "Label/M", INK)?;
    k.chip(actions, "Start free", ["Label/M", ON_BRAND], PRIMARY, [10.0, 18.0], 12.0).map(drop)
}

fn hero(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let hero = section(k, root, "Hero", l, l.row(), [88.0, 88.0, if l.narrow { 32.0 } else { 72.0 }], json!({}))?;
    k.set(hero, json!({"crossAlign": "center"}))?;
    let words = k.stack(hero, "Words", V, 24.0, json!(0), json!({"sizingH": "fill"}))?;
    k.chip(words, "New · Shared trail maps", ["Label/S", PRIMARY], SOFT, [6.0, 12.0], 999.0)?;
    let text = k.stack(words, "Headline", V, 16.0, json!(0), json!({"sizingH": "fill"}))?;
    k.para(text, "Plan every walk together", if l.narrow { "Display" } else { "Display/XL" }, INK)?;
    k.para(
        text,
        "Ridgeline Studio keeps your club's routes, maps and meeting points in one place, so the only thing left to decide is who brings the flask.",
        "Body/L",
        MUTED,
    )?;
    let actions = k.stack(words, "Actions", H, 12.0, json!(0), json!({}))?;
    button(k, actions, "Start free", true)?;
    button(k, actions, "Watch the tour", false)?;
    k.text(words, "Free for clubs of up to ten people", "Body/S", MUTED)?;
    let picture = if l.narrow { json!({"cornerRadius": 24, "sizingH": "fill"}) } else { json!({"cornerRadius": 32}) };
    k.scene(hero, "Picture", if l.narrow { [350.0, 260.0] } else { [520.0, 420.0] }, picture).map(drop)
}

/// A section's centred title and the line under it.
fn heading(k: &mut Kit, parent: NodeId, title: &str, line: &str) -> Result<()> {
    let head = k.stack(parent, "Heading", V, 12.0, json!(0), json!({"sizingH": "fill", "maxWidth": 640, "crossAlign": "center"}))?;
    let center = json!({"sizingH": "fill", "textAutoResize": "height", "textAlign": "center"});
    k.text_with(head, title, "Heading/L", INK, center.clone())?;
    k.text_with(head, line, "Body/L", MUTED, center).map(drop)
}

fn features(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let features = section(k, root, "Features", l, V, [88.0, 88.0, 48.0], json!({"fill": PAGE}))?;
    k.set(features, json!({"crossAlign": "center"}))?;
    heading(k, features, "Everything the day needs", "Three things walking clubs told us they were tired of doing by hand.")?;
    let cards = k.stack(features, "Cards", l.row(), 24.0, json!(0), json!({"sizingH": "fill"}))?;
    for (title, glyph, line) in &FEATURES {
        let card = k.stack(cards, title, V, 16.0, json!(28), json!({"fill": CARD, "cornerRadius": 24, "sizingH": "fill", "effects": shadow()}))?;
        let tile = k.tile(card, 48.0, 14.0)?;
        k.icon(tile, glyph, ICON, PRIMARY)?;
        let words = k.stack(card, "Words", V, 8.0, json!(0), json!({"sizingH": "fill"}))?;
        k.para(words, title, "Heading/S", INK)?;
        k.para(words, line, "Body/M", MUTED)?;
    }
    Ok(())
}

fn figures(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let band = section(k, root, "Figures", l, l.row(), [64.0, 64.0, 32.0], json!({"fill": PRIMARY}))?;
    for (value, label) in FIGURES {
        let figure = k.stack(band, label, V, 4.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
        k.text(figure, value, "Display", ON_BRAND)?;
        k.text_with(figure, label, "Body/M", ON_BRAND, json!({"opacity": 0.8}))?;
    }
    Ok(())
}

fn quote(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let quote = section(k, root, "Quote", l, V, [88.0, 88.0, 28.0], json!({}))?;
    k.set(quote, json!({"crossAlign": "center"}))?;
    let words = json!({"sizingH": "fill", "maxWidth": 760, "textAutoResize": "height", "textAlign": "center"});
    let said = "“We used to plan in three group chats and a spreadsheet. Now the route is simply there, and everyone has it.”";
    k.text_with(quote, said, "Heading/M", INK, words)?;
    let person = k.stack(quote, "Person", H, 12.0, json!(0), json!({"crossAlign": "center"}))?;
    let avatar = k.tile(person, 44.0, 22.0)?;
    k.text(avatar, "IO", "Label/M", PRIMARY)?;
    let who = k.stack(person, "Who", V, 0.0, json!(0), json!({}))?;
    k.text(who, "Ilse Okafor", "Label/M", INK)?;
    k.text(who, "Walk leader, Harrow Fell Ramblers", "Body/S", MUTED).map(drop)
}

fn call_to_action(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let outer = section(k, root, "Call to action", l, V, [0.0, 88.0, 0.0], json!({}))?;
    let pad = if l.narrow { 28.0 } else { 56.0 };
    let props = json!({"fill": SOFT, "cornerRadius": 28, "sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": l.cross()});
    let card = k.stack(outer, "Card", l.row(), 24.0, json!(pad), props)?;
    let words = k.stack(card, "Words", V, 8.0, json!(0), json!({"sizingH": "fill"}))?;
    k.para(words, "Take the next walk off the group chat", "Heading/L", INK)?;
    k.para(words, "Set up your club in a couple of minutes. No card needed.", "Body/L", MUTED)?;
    button(k, card, "Start free", true).map(drop)
}

fn footer(k: &mut Kit, root: NodeId, l: &Layout) -> Result<()> {
    let footer = section(k, root, "Footer", l, l.row(), [32.0, 32.0, 12.0], json!({"stroke": BORDER, "strokeSides": TOP_EDGE}))?;
    k.set(footer, json!({"mainAlign": "spaceBetween", "crossAlign": l.cross()}))?;
    logo(k, footer)?;
    k.text(footer, "© 2026 Ridgeline Studio, an invented company", "Body/S", MUTED).map(drop)
}
