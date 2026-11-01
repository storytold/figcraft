//! Wireframe kit: greyscale components (buttons, an input, an image placeholder, a paragraph, an
//! avatar, a list item, a card, a navigation bar and a tab bar) and two screens sketched with
//! instances of them, a phone and a desktop one.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::glyphs;
use super::kit::{BOTTOM_EDGE, H, Kit, TOP_EDGE, TYPE, V, merged};
use crate::Result;

const WIRE_INK: &str = "Wire/Ink";
const MID: &str = "Wire/Mid";
const LINE: &str = "Wire/Line";
const FILL: &str = "Wire/Fill";
const PAPER: &str = "Wire/Paper";

struct Parts {
    button: NodeId,
    input: NodeId,
    image: NodeId,
    paragraph: NodeId,
    avatar: NodeId,
    item: NodeId,
    card: NodeId,
    nav: NodeId,
    tabs: NodeId,
}

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.colors(&[(WIRE_INK, "#2B2B2B"), (MID, "#8A8A8A"), (LINE, "#C4C4C4"), (FILL, "#EBEBEB"), (PAPER, "#FFFFFF")])?;
    k.type_styles(TYPE)?;
    k.rename_page("Wireframes")?;
    let screens = k.page()?;
    let page = k.add_page("Components")?;
    let (button, input, image, paragraph, avatar) = (buttons(k, page)?, input(k, page)?, image(k, page)?, paragraph(k, page)?, avatar(k, page)?);
    let (item, card) = (list_item(k, page, avatar)?, card(k, page, [image, paragraph, button])?);
    let parts = Parts { button, input, image, paragraph, avatar, item, card, nav: nav(k, page, button)?, tabs: tabs(k, page)? };
    phone(k, screens, &parts)?;
    desktop(k, screens, &parts)
}

fn fill() -> Value {
    json!({"sizingH": "fill"})
}

/// A component that stacks its content at (`x`, `y`) on the components page.
fn part(k: &mut Kit, page: NodeId, name: &str, dir: &str, [gap, x, y]: [f64; 3], pad: Value, props: Value) -> Result<NodeId> {
    k.stack_of("component", page, name, dir, gap, pad, merged(json!({"x": x, "y": y}), props))
}

/// A grey bar standing in for a line of text.
fn bar(k: &mut Kit, parent: NodeId, props: Value) -> Result<NodeId> {
    k.rect(parent, "Line", [0.0, 0.0, 120.0, 8.0], merged(json!({"fill": LINE, "cornerRadius": 4}), props))
}

fn buttons(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let mut variants = Vec::new();
    let mut labels = Vec::new();
    for (i, kind) in ["Primary", "Secondary"].into_iter().enumerate() {
        let (fill, ink) = if i == 0 { (WIRE_INK, PAPER) } else { (PAPER, WIRE_INK) };
        let props = json!({"fill": fill, "stroke": WIRE_INK, "strokeWeight": 1, "cornerRadius": 6, "mainAlign": "center", "crossAlign": "center"});
        let c = part(k, page, &format!("Kind={kind}"), H, [8.0, i as f64 * 140.0, 0.0], json!([10, 18, 10, 18]), props)?;
        labels.push(k.text(c, "Button", "Label/M", ink)?);
        variants.push(c);
    }
    let set = k.variants("Button", &variants, json!({}))?;
    k.text_prop(set, "Label", "Button", &labels)?;
    Ok(set)
}

fn input(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let c = part(k, page, "Input", V, [6.0, 0.0, 120.0], json!(0), json!({"sizingH": "fixed", "width": 260}))?;
    let label = k.text(c, "Label", "Label/S", WIRE_INK)?;
    let field = json!({"fill": PAPER, "stroke": MID, "strokeWeight": 1, "cornerRadius": 6, "sizingH": "fill"});
    let field = k.stack(c, "Field", H, 8.0, json!([10, 12, 10, 12]), field)?;
    let hint = k.text(field, "Placeholder", "Body/M", MID)?;
    k.text_prop(c, "Label", "Label", &[label])?;
    k.text_prop(c, "Placeholder", "Placeholder", &[hint])?;
    Ok(c)
}

/// An image placeholder: a grey box crossed corner to corner, whatever its size.
fn image(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let c = k.node("component", page, "Image", [0.0, 220.0, 240.0, 160.0], json!({"fill": FILL, "stroke": LINE, "strokeWeight": 1, "clip": true}))?;
    for path in ["M0 0 L240 160", "M240 0 L0 160"] {
        let cross = k.line(c, "Cross", path, LINE, 1.0)?;
        k.set(cross, json!({"constraints": {"horizontal": "scale", "vertical": "scale"}}))?;
    }
    Ok(c)
}

fn paragraph(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let c = part(k, page, "Paragraph", V, [8.0, 300.0, 120.0], json!(0), json!({"sizingH": "fixed", "width": 240}))?;
    bar(k, c, fill())?;
    bar(k, c, fill())?;
    bar(k, c, json!({"width": 150}))?;
    Ok(c)
}

fn avatar(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let c = k.node("component", page, "Avatar", [300.0, 0.0, 40.0, 40.0], json!({"fills": [], "clip": false}))?;
    let scale = json!({"horizontal": "scale", "vertical": "scale"});
    k.ellipse(c, "Circle", [0.0, 0.0, 40.0, 40.0], json!({"fill": FILL, "stroke": MID, "strokeWeight": 1, "constraints": scale}))?;
    Ok(c)
}

fn list_item(k: &mut Kit, page: NodeId, avatar: NodeId) -> Result<NodeId> {
    let props = json!({"sizingH": "fixed", "width": 342, "crossAlign": "center", "stroke": LINE, "strokeSides": BOTTOM_EDGE});
    let c = part(k, page, "List item", H, [12.0, 0.0, 420.0], json!([12, 0, 12, 0]), props)?;
    k.instance(avatar, c, json!({}), json!({}))?;
    let words = k.stack(c, "Words", V, 8.0, json!(0), fill())?;
    let title = k.text(words, "List item", "Label/M", WIRE_INK)?;
    bar(k, words, json!({"width": 160}))?;
    k.icon(c, &glyphs::CHEVRON_RIGHT, 20.0, MID)?;
    k.text_prop(c, "Title", "List item", &[title])?;
    Ok(c)
}

fn card(k: &mut Kit, page: NodeId, [image, paragraph, button]: [NodeId; 3]) -> Result<NodeId> {
    let props = json!({"fill": PAPER, "stroke": LINE, "strokeWeight": 1, "cornerRadius": 8, "sizingH": "fixed", "width": 280});
    let c = part(k, page, "Card", V, [14.0, 400.0, 220.0], json!(14), props)?;
    k.instance(image, c, json!({}), json!({"sizingH": "fill", "height": 132}))?;
    let title = k.text(c, "Card title", "Heading/S", WIRE_INK)?;
    k.instance(paragraph, c, json!({}), fill())?;
    k.instance(button, c, json!({"Kind": "Secondary", "Label": "Action"}), json!({}))?;
    k.text_prop(c, "Title", "Card title", &[title])?;
    Ok(c)
}

fn nav(k: &mut Kit, page: NodeId, button: NodeId) -> Result<NodeId> {
    let props = json!({"fill": PAPER, "sizingH": "fixed", "width": 1280, "mainAlign": "spaceBetween", "crossAlign": "center", "stroke": LINE, "strokeSides": BOTTOM_EDGE});
    let c = part(k, page, "Navigation bar", H, [24.0, 0.0, 620.0], json!([16, 40, 16, 40]), props)?;
    k.rect(c, "Logo", [0.0, 0.0, 36.0, 36.0], json!({"fill": WIRE_INK, "cornerRadius": 8}))?;
    let links = k.stack(c, "Links", H, 36.0, json!(0), json!({}))?;
    for _ in 0..4 {
        k.text(links, "Link", "Label/M", MID)?;
    }
    k.instance(button, c, json!({"Label": "Action"}), json!({}))?;
    Ok(c)
}

fn tabs(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let props = json!({"fill": PAPER, "sizingH": "fixed", "width": 390, "stroke": LINE, "strokeSides": TOP_EDGE});
    let c = part(k, page, "Tab bar", H, [0.0, 0.0, 510.0], json!([10, 12, 26, 12]), props)?;
    for i in 0..4 {
        let tab = k.stack(c, "Tab", V, 6.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
        let ink = if i == 0 { WIRE_INK } else { LINE };
        k.rect(tab, "Icon", [0.0, 0.0, 24.0, 24.0], json!({"fill": ink, "cornerRadius": 6}))?;
        k.text(tab, "Tab", "Label/S", if i == 0 { WIRE_INK } else { MID })?;
    }
    Ok(c)
}

/// A screen: a white frame of a fixed size that stacks its content.
fn screen(k: &mut Kit, page: NodeId, name: &str, [x, w, h]: [f64; 3]) -> Result<NodeId> {
    let props = json!({"fill": PAPER, "stroke": LINE, "strokeWeight": 1, "strokeAlign": "outside", "clip": true, "sizingH": "fixed", "sizingV": "fixed", "width": w, "height": h, "x": x});
    k.stack(page, name, V, 0.0, json!(0), props)
}

fn phone(k: &mut Kit, page: NodeId, p: &Parts) -> Result<()> {
    let s = screen(k, page, "Phone · List", [0.0, 390.0, 844.0])?;
    let body = k.stack(s, "Content", V, 20.0, json!([56, 24, 24, 24]), json!({"sizingH": "fill", "sizingV": "fill"}))?;
    let header = k.stack(body, "Header", H, 12.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    k.text(header, "Screen title", "Heading/M", WIRE_INK)?;
    k.instance(p.avatar, header, json!({}), json!({}))?;
    k.instance(p.input, body, json!({"Label": "Search", "Placeholder": "Type to search"}), fill())?;
    let list = k.stack(body, "List", V, 0.0, json!(0), fill())?;
    for title in ["First item", "Second item", "Third item"] {
        k.instance(p.item, list, json!({"Title": title}), fill())?;
    }
    k.instance(p.card, body, json!({"Title": "Featured"}), fill())?;
    k.instance(p.tabs, s, json!({}), fill()).map(drop)
}

fn desktop(k: &mut Kit, page: NodeId, p: &Parts) -> Result<()> {
    let s = screen(k, page, "Desktop · Home", [470.0, 1280.0, 900.0])?;
    k.instance(p.nav, s, json!({}), fill())?;
    let body = k.stack(s, "Content", V, 48.0, json!([56, 80, 56, 80]), json!({"sizingH": "fill", "sizingV": "fill"}))?;
    let hero = k.stack(body, "Hero", H, 64.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
    let words = k.stack(hero, "Words", V, 24.0, json!(0), fill())?;
    k.text_with(words, "Headline goes here", "Display", WIRE_INK, json!({"sizingH": "fill", "textAutoResize": "height"}))?;
    k.instance(p.paragraph, words, json!({}), fill())?;
    let actions = k.stack(words, "Actions", H, 12.0, json!(0), json!({}))?;
    k.instance(p.button, actions, json!({"Label": "Primary action"}), json!({}))?;
    k.instance(p.button, actions, json!({"Kind": "Secondary", "Label": "Secondary"}), json!({}))?;
    k.instance(p.image, hero, json!({}), json!({"width": 520, "height": 300}))?;
    let cards = k.stack(body, "Cards", H, 24.0, json!(0), fill())?;
    for title in ["First card", "Second card", "Third card", "Fourth card"] {
        k.instance(p.card, cards, json!({"Title": title}), fill())?;
    }
    Ok(())
}
