//! Mobile app: four screens of an invented trail app ("Ridgeline Trails") built with auto layout
//! from three components (a button, a trail card, a tab bar with two variants), linked into a
//! prototype flow.

use figcraft_doc::NodeId;
use serde_json::json;

use super::glyphs::{self, Glyph};
use super::kit::{BORDER, CARD, H, ICON, INK, Kit, MUTED, ON_BRAND, PAGE, PRIMARY, SUNKEN, TOP_EDGE, V, shadow};
use crate::Result;

const W: f64 = 390.0;
const SCREEN_H: f64 = 844.0;
/// Space between screens on the canvas.
const GAP: f64 = 80.0;

/// The trails on the Explore screen: (name, distance and difficulty).
const TRAILS: [(&str, &str); 3] =
    [("Cedar Hollow Loop", "8.4 km · Moderate"), ("Lantern Ridge", "12.1 km · Hard"), ("Mill Pond Walk", "3.2 km · Easy")];

const TABS: [(&str, Glyph); 4] = [("Explore", glyphs::MOUNTAIN), ("Saved", glyphs::BOOKMARK), ("Activity", glyphs::CHART), ("Profile", glyphs::USER)];

struct Parts {
    button: NodeId,
    card: NodeId,
    tabs: NodeId,
}

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.rename_page("Screens")?;
    let screens = k.page()?;
    let page = k.add_page("Components")?;
    let parts = Parts { button: button(k, page)?, card: trail_card(k, page)?, tabs: tab_bar(k, page)? };
    k.go(screens)?;
    let (welcome, start) = welcome(k, screens, &parts)?;
    let (explore, first_card, explore_tabs) = explore(k, screens, &parts)?;
    let (detail, back) = detail(k, screens, &parts)?;
    let (profile, profile_tabs) = profile(k, screens, &parts)?;
    for (from, to) in [(start, explore), (first_card, detail), (back, explore), (explore_tabs, profile), (profile_tabs, explore)] {
        k.link(from, to)?;
    }
    k.flow(welcome, "Onboarding")
}

fn button(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let props = json!({"fill": PRIMARY, "cornerRadius": 16, "mainAlign": "center", "crossAlign": "center"});
    let c = k.stack_of("component", page, "Button", H, 8.0, json!([16, 24, 16, 24]), props)?;
    let label = k.text(c, "Button", "Label/M", ON_BRAND)?;
    k.text_prop(c, "Label", "Button", &[label])?;
    Ok(c)
}

fn trail_card(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let props =
        json!({"fill": CARD, "cornerRadius": 20, "crossAlign": "center", "effects": shadow(), "sizingH": "fixed", "width": W - 48.0, "y": 120});
    let c = k.stack_of("component", page, "Trail card", H, 14.0, json!(12), props)?;
    k.scene(c, "Picture", [72.0, 72.0], json!({"cornerRadius": 14}))?;
    let words = k.stack(c, "Words", V, 2.0, json!(0), json!({"sizingH": "fill"}))?;
    let title = k.text(words, "Trail name", "Heading/S", INK)?;
    let meta = k.text(words, "Distance · Difficulty", "Body/S", MUTED)?;
    k.icon(c, &glyphs::CHEVRON_RIGHT, 20.0, MUTED)?;
    k.text_prop(c, "Name", "Trail name", &[title])?;
    k.text_prop(c, "Details", "Distance · Difficulty", &[meta])?;
    Ok(c)
}

/// The tab bar as a component set: one variant per tab the screens show as current.
fn tab_bar(k: &mut Kit, page: NodeId) -> Result<NodeId> {
    let mut variants = Vec::new();
    for (i, current) in ["Explore", "Profile"].into_iter().enumerate() {
        let props = json!({"fill": CARD, "stroke": BORDER, "strokeSides": TOP_EDGE, "sizingH": "fixed", "width": W, "y": 260.0 + i as f64 * 110.0});
        let bar = k.stack_of("component", page, &format!("Current={current}"), H, 0.0, json!([10, 12, 26, 12]), props)?;
        for (label, glyph) in &TABS {
            let color = if *label == current { PRIMARY } else { MUTED };
            let tab = k.stack(bar, label, V, 4.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
            k.icon(tab, glyph, ICON, color)?;
            k.text(tab, label, "Label/S", color)?;
        }
        variants.push(bar);
    }
    k.variants("Tab bar", &variants, json!({}))
}

/// A screen: a phone-sized frame that stacks its content.
fn screen(k: &mut Kit, page: NodeId, name: &str, index: usize) -> Result<NodeId> {
    let props = json!({"fill": PAGE, "stroke": BORDER, "strokeWeight": 1, "strokeAlign": "outside", "clip": true, "sizingH": "fixed", "sizingV": "fixed", "width": W, "height": SCREEN_H, "x": index as f64 * (W + GAP)});
    k.stack(page, name, V, 0.0, json!(0), props)
}

/// The part of a screen above the tab bar or its last button: it takes the room that is left.
fn body(k: &mut Kit, screen: NodeId, gap: f64, top: f64) -> Result<NodeId> {
    k.stack(screen, "Content", V, gap, json!([top, 24, 24, 24]), json!({"sizingH": "fill", "sizingV": "fill"}))
}

fn welcome(k: &mut Kit, page: NodeId, parts: &Parts) -> Result<(NodeId, NodeId)> {
    let s = screen(k, page, "Welcome", 0)?;
    k.scene(s, "Picture", [W, 440.0], json!({"sizingH": "fill"}))?;
    let content = body(k, s, 0.0, 32.0)?;
    k.set(content, json!({"mainAlign": "spaceBetween"}))?;
    let words = k.stack(content, "Words", V, 12.0, json!(0), json!({"sizingH": "fill"}))?;
    k.text(words, "RIDGELINE TRAILS", "Label/S", PRIMARY)?;
    k.para(words, "Find your next trail", "Heading/L", INK)?;
    k.para(words, "Maps, distances and notes for quiet walks and long climbs, saved for when there is no signal.", "Body/M", MUTED)?;
    let actions = k.stack(content, "Actions", V, 16.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
    let start = k.instance(parts.button, actions, json!({"Label": "Get started"}), json!({"sizingH": "fill"}))?;
    k.text(actions, "I already have an account", "Label/M", PRIMARY)?;
    Ok((s, start))
}

fn avatar(k: &mut Kit, parent: NodeId, size: f64, style: &str) -> Result<NodeId> {
    let a = k.tile(parent, size, size / 2.0)?;
    k.text(a, "ML", style, PRIMARY)?;
    Ok(a)
}

fn explore(k: &mut Kit, page: NodeId, parts: &Parts) -> Result<(NodeId, NodeId, NodeId)> {
    let s = screen(k, page, "Explore", 1)?;
    let content = body(k, s, 20.0, 56.0)?;
    let header = k.stack(content, "Header", H, 12.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    let hello = k.stack(header, "Greeting", V, 2.0, json!(0), json!({}))?;
    k.text(hello, "Good morning", "Body/S", MUTED)?;
    k.text(hello, "Mara Lindqvist", "Heading/M", INK)?;
    avatar(k, header, 44.0, "Label/M")?;
    let search_props = json!({"fill": CARD, "cornerRadius": 16, "stroke": BORDER, "strokeWeight": 1, "sizingH": "fill", "crossAlign": "center"});
    let search = k.stack(content, "Search", H, 10.0, json!([14, 16, 14, 16]), search_props)?;
    k.icon(search, &glyphs::SEARCH, 20.0, MUTED)?;
    k.text(search, "Search trails and places", "Body/M", MUTED)?;
    let filters = k.stack(content, "Filters", H, 8.0, json!(0), json!({}))?;
    for (i, label) in ["Nearby", "Easy", "Lakes", "Summits"].into_iter().enumerate() {
        let (color, fill) = if i == 0 { (ON_BRAND, PRIMARY) } else { (INK, SUNKEN) };
        k.chip(filters, label, ["Label/M", color], fill, [8.0, 16.0], 999.0)?;
    }
    let title =
        k.stack(content, "Section title", H, 8.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    k.text(title, "Popular this week", "Heading/S", INK)?;
    k.text(title, "See all", "Label/M", PRIMARY)?;
    let list = k.stack(content, "Trails", V, 12.0, json!(0), json!({"sizingH": "fill"}))?;
    let mut first = None;
    for (name, details) in TRAILS {
        let card = k.instance(parts.card, list, json!({"Name": name, "Details": details}), json!({"sizingH": "fill"}))?;
        first.get_or_insert(card);
    }
    let tabs = k.instance(parts.tabs, s, json!({"Current": "Explore"}), json!({"sizingH": "fill"}))?;
    Ok((s, first.unwrap_or(list), tabs))
}

fn detail(k: &mut Kit, page: NodeId, parts: &Parts) -> Result<(NodeId, NodeId)> {
    let s = screen(k, page, "Trail", 2)?;
    k.scene(s, "Picture", [W, 300.0], json!({"sizingH": "fill"}))?;
    let back = k.tile(s, 44.0, 22.0)?;
    k.set(back, json!({"name": "Back", "fill": CARD, "absolute": true, "x": 20, "y": 52}))?;
    k.icon(back, &glyphs::ARROW_LEFT, 20.0, INK)?;
    let content = body(k, s, 20.0, 24.0)?;
    k.set(content, json!({"mainAlign": "spaceBetween"}))?;
    let about = k.stack(content, "About", V, 16.0, json!(0), json!({"sizingH": "fill"}))?;
    let heading = k.stack(about, "Heading", V, 6.0, json!(0), json!({"sizingH": "fill"}))?;
    let [(name, _), ..] = TRAILS;
    k.para(heading, name, "Heading/L", INK)?;
    let place = k.stack(heading, "Place", H, 6.0, json!(0), json!({"crossAlign": "center"}))?;
    k.icon(place, &glyphs::PIN, 16.0, MUTED)?;
    k.text(place, "Northfork Valley", "Body/M", MUTED)?;
    let stats = k.stack(about, "Figures", H, 10.0, json!(0), json!({"sizingH": "fill"}))?;
    for (value, label) in [("8.4 km", "Distance"), ("310 m", "Climb"), ("2 h 40", "Time")] {
        k.stat(stats, value, label, "Heading/S")?;
    }
    k.text(about, "About this trail", "Heading/S", INK)?;
    k.para(about, "A shaded loop through old cedars that opens onto the ridge at the halfway point. Bring water: the spring near the trailhead dries up in late summer.", "Body/M", MUTED)?;
    k.instance(parts.button, content, json!({"Label": "Start hike"}), json!({"sizingH": "fill"}))?;
    Ok((s, back))
}

fn profile(k: &mut Kit, page: NodeId, parts: &Parts) -> Result<(NodeId, NodeId)> {
    let s = screen(k, page, "Profile", 3)?;
    let content = body(k, s, 24.0, 72.0)?;
    let who = k.stack(content, "Who", V, 6.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
    avatar(k, who, 88.0, "Heading/M")?;
    k.text(who, "Mara Lindqvist", "Heading/M", INK)?;
    k.text(who, "Hiking since 2021 · Northfork", "Body/M", MUTED)?;
    let stats = k.stack(content, "Figures", H, 10.0, json!(0), json!({"sizingH": "fill"}))?;
    for (value, label) in [("128 km", "This year"), ("23", "Hikes"), ("9", "Summits")] {
        k.stat(stats, value, label, "Heading/S")?;
    }
    let menu = k.stack(content, "Menu", V, 0.0, json!([4, 16, 4, 16]), json!({"fill": CARD, "cornerRadius": 20, "sizingH": "fill"}))?;
    for (label, glyph) in
        [("Saved trails", glyphs::BOOKMARK), ("Offline maps", glyphs::DOWNLOAD), ("Notifications", glyphs::BELL), ("Settings", glyphs::SLIDERS)]
    {
        let row = k.stack(menu, label, H, 12.0, json!([14, 0, 14, 0]), json!({"sizingH": "fill", "crossAlign": "center"}))?;
        k.icon(row, &glyph, 20.0, PRIMARY)?;
        k.text_with(row, label, "Body/M", INK, json!({"sizingH": "fill"}))?;
        k.icon(row, &glyphs::CHEVRON_RIGHT, 16.0, MUTED)?;
    }
    let tabs = k.instance(parts.tabs, s, json!({"Current": "Profile"}), json!({"sizingH": "fill"}))?;
    Ok((s, tabs))
}
