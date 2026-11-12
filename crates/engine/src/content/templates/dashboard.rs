//! Dashboard: one analytics screen of an invented booking product ("Ridgeline Analytics"): a
//! sidebar, then figures, charts and a table placed on a grid auto layout. The charts are drawn
//! from vectors, rectangles and arcs that follow their plot's size.

use figcraft_doc::NodeId;
use serde_json::{Value, json};

use super::glyphs::{self, Glyph};
use super::kit::{ACCENT, BORDER, BOTTOM_EDGE, CARD, H, ICON, INK, Kit, MUTED, ON_BRAND, PAGE, PRIMARY, RIGHT_EDGE, SOFT, V, merged};
use crate::Result;

const NAV: [(&str, Glyph); 6] = [
    ("Overview", glyphs::CHART),
    ("Bookings", glyphs::CALENDAR),
    ("Routes", glyphs::MOUNTAIN),
    ("Guides", glyphs::USER),
    ("Messages", glyphs::MAIL),
    ("Settings", glyphs::SLIDERS),
];

/// The figures across the top: (label, value, change, whether the change is good).
const FIGURES: [(&str, &str, &str, bool); 4] = [
    ("Revenue", "€48,260", "+12.4%", true),
    ("Bookings", "1,284", "+8.1%", true),
    ("Average group", "6.3", "+0.4", true),
    ("Cancellations", "3.2%", "+0.6%", false),
];

/// Revenue per week, as a share of the plot's height.
const REVENUE: [f64; 12] = [0.32, 0.38, 0.35, 0.46, 0.52, 0.48, 0.61, 0.58, 0.70, 0.66, 0.78, 0.86];
const LAST_YEAR: [f64; 12] = [0.26, 0.30, 0.33, 0.36, 0.41, 0.44, 0.47, 0.52, 0.55, 0.57, 0.62, 0.66];
const WEEKDAYS: [(&str, f64); 7] = [("Mon", 0.42), ("Tue", 0.36), ("Wed", 0.48), ("Thu", 0.55), ("Fri", 0.74), ("Sat", 0.96), ("Sun", 0.82)];
/// Bookings by kind of walk: (label, share, colour).
const KINDS: [(&str, f64, &str); 3] = [("Day walks", 0.54, PRIMARY), ("Weekends", 0.31, ACCENT), ("Courses", 0.15, SEA)];
const SEA: &str = "#6FB09B";
const BAD: [&str; 2] = ["#B4472E", "#FBE9E4"];

const BOOKINGS: [[&str; 5]; 4] = [
    ["Cedar Hollow Loop", "Ilse Okafor", "14 June", "Confirmed", "€420"],
    ["Lantern Ridge at dawn", "Tomas Beck", "15 June", "Confirmed", "€680"],
    ["Mill Pond family walk", "Mara Lindqvist", "15 June", "Waiting", "€150"],
    ["Harrow Fell weekend", "Ilse Okafor", "21 June", "Confirmed", "€1,940"],
];

/// A chart's plot is drawn at this size, then follows its card.
const PLOT: [f64; 2] = [800.0, 220.0];

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.rename_page("Dashboard")?;
    let page = k.page()?;
    let frame = json!({"fill": PAGE, "stroke": BORDER, "strokeWeight": 1, "strokeAlign": "outside", "clip": true, "sizingH": "fixed", "sizingV": "fixed", "width": 1440, "height": 960});
    let root = k.stack(page, "Dashboard", H, 0.0, json!(0), frame)?;
    sidebar(k, root)?;
    let main = k.stack(root, "Main", V, 24.0, json!(32), json!({"sizingH": "fill", "sizingV": "fill"}))?;
    top_bar(k, main)?;
    let grid = json!({"layout": "grid", "gridColumns": 4, "gridRows": 3, "columnTracks": ["1fr", "1fr", "1fr", "1fr"], "rowTracks": ["auto", "1fr", "1fr"], "crossGap": 20, "sizingH": "fill", "sizingV": "fill"});
    let grid = k.stack(main, "Grid", H, 20.0, json!(0), grid)?;
    for (label, value, change, good) in FIGURES {
        figure(k, grid, label, value, change, good)?;
    }
    revenue(k, grid)?;
    kinds(k, grid)?;
    weekdays(k, grid)?;
    bookings(k, grid)
}

fn sidebar(k: &mut Kit, root: NodeId) -> Result<()> {
    let props = json!({"fill": CARD, "stroke": BORDER, "strokeSides": RIGHT_EDGE, "sizingH": "fixed", "width": 248, "sizingV": "fill", "mainAlign": "spaceBetween"});
    let bar = k.stack(root, "Sidebar", V, 32.0, json!([28, 16, 20, 16]), props)?;
    let top = k.stack(bar, "Top", V, 28.0, json!(0), json!({"sizingH": "fill"}))?;
    let logo = k.stack(top, "Logo", H, 8.0, json!([0, 12, 0, 12]), json!({"crossAlign": "center"}))?;
    k.icon(logo, &glyphs::MOUNTAIN, ICON, PRIMARY)?;
    k.text(logo, "Ridgeline Analytics", "Heading/S", INK)?;
    let nav = k.stack(top, "Navigation", V, 4.0, json!(0), json!({"sizingH": "fill"}))?;
    for (i, (label, glyph)) in NAV.iter().enumerate() {
        let (color, style, props) = if i == 0 { (PRIMARY, "Label/M", json!({"fill": SOFT})) } else { (MUTED, "Body/M", json!({})) };
        let props = merged(json!({"sizingH": "fill", "cornerRadius": 12, "crossAlign": "center"}), props);
        let item = k.stack(nav, label, H, 12.0, json!([10, 12, 10, 12]), props)?;
        k.icon(item, glyph, 20.0, color)?;
        k.text(item, label, style, color)?;
    }
    let user = k.stack(bar, "Account", H, 12.0, json!(12), json!({"fill": PAGE, "cornerRadius": 16, "sizingH": "fill", "crossAlign": "center"}))?;
    let avatar = k.tile(user, 36.0, 18.0)?;
    k.text(avatar, "TB", "Label/S", PRIMARY)?;
    let who = k.stack(user, "Who", V, 0.0, json!(0), json!({"sizingH": "fill"}))?;
    k.text(who, "Tomas Beck", "Label/M", INK)?;
    k.text(who, "Operations", "Body/S", MUTED).map(drop)
}

fn top_bar(k: &mut Kit, main: NodeId) -> Result<()> {
    let bar = k.stack(main, "Top bar", H, 16.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    let title = k.stack(bar, "Title", V, 2.0, json!(0), json!({}))?;
    k.text(title, "Overview", "Heading/L", INK)?;
    k.text(title, "Bookings and revenue, 1 March to 31 May", "Body/M", MUTED)?;
    let tools = k.stack(bar, "Tools", H, 12.0, json!(0), json!({"crossAlign": "center"}))?;
    let field =
        json!({"fill": CARD, "cornerRadius": 14, "stroke": BORDER, "strokeWeight": 1, "sizingH": "fixed", "width": 260, "crossAlign": "center"});
    let search = k.stack(tools, "Search", H, 10.0, json!([11, 14, 11, 14]), field)?;
    k.icon(search, &glyphs::SEARCH, 20.0, MUTED)?;
    k.text(search, "Search bookings", "Body/M", MUTED)?;
    let bell = k.tile(tools, 44.0, 14.0)?;
    k.set(bell, json!({"name": "Notifications", "fill": CARD, "stroke": BORDER, "strokeWeight": 1}))?;
    k.icon(bell, &glyphs::BELL, 20.0, INK)?;
    k.chip(tools, "Export", ["Label/M", ON_BRAND], PRIMARY, [12.0, 18.0], 14.0).map(drop)
}

/// A card in the grid, `span` columns wide, holding a title row and what follows.
fn card(k: &mut Kit, grid: NodeId, name: &str, span: usize, note: &str) -> Result<NodeId> {
    let props = json!({"fill": CARD, "cornerRadius": 20, "sizingH": "fill", "sizingV": "fill", "gridColumnSpan": span});
    let card = k.stack(grid, name, V, 16.0, json!(20), props)?;
    let head = k.stack(card, "Heading", H, 8.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    k.text(head, name, "Heading/S", INK)?;
    k.text(head, note, "Body/S", MUTED)?;
    Ok(card)
}

fn figure(k: &mut Kit, grid: NodeId, label: &str, value: &str, change: &str, good: bool) -> Result<()> {
    let card = k.stack(grid, label, V, 8.0, json!(20), json!({"fill": CARD, "cornerRadius": 20, "sizingH": "fill"}))?;
    k.text(card, label, "Body/S", MUTED)?;
    let row = k.stack(card, "Value", H, 8.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween", "crossAlign": "center"}))?;
    k.text(row, value, "Heading/L", INK)?;
    let [color, fill] = if good { [PRIMARY, SOFT] } else { BAD };
    k.chip(row, change, ["Label/S", color], fill, [4.0, 10.0], 999.0).map(drop)
}

/// A plot area in `card` that fills what room is left; its shapes follow its size.
fn plot(k: &mut Kit, card: NodeId, size: [f64; 2]) -> Result<NodeId> {
    k.frame(card, "Plot", [0.0, 0.0, size[0], size[1]], json!({"fills": [], "clip": false}))
}

/// Make `plot`'s children follow its size, then let it fill its card.
fn fit(k: &mut Kit, plot: NodeId, shapes: &[NodeId]) -> Result<()> {
    for id in shapes {
        k.set(*id, json!({"constraints": {"horizontal": "scale", "vertical": "scale"}}))?;
    }
    k.set(plot, json!({"sizingH": "fill", "sizingV": "fill"}))
}

/// Labels under a plot, spread across its width.
fn axis(k: &mut Kit, card: NodeId, labels: &[&str]) -> Result<()> {
    let row = k.stack(card, "Axis", H, 0.0, json!(0), json!({"sizingH": "fill", "mainAlign": "spaceBetween"}))?;
    for label in labels {
        k.text(row, label, "Label/S", MUTED)?;
    }
    Ok(())
}

/// A polyline through `values` (shares of the height) across the plot, as path data.
fn polyline(values: &[f64], [w, h]: [f64; 2]) -> String {
    let step = w / (values.len().max(2) - 1) as f64;
    values.iter().enumerate().map(|(i, v)| format!("{}{:.1} {:.1}", if i == 0 { "M" } else { " L" }, i as f64 * step, h - v * h)).collect()
}

fn revenue(k: &mut Kit, grid: NodeId) -> Result<()> {
    let card = card(k, grid, "Revenue", 3, "This year and last year, by week")?;
    let p = plot(k, card, PLOT)?;
    let [w, h] = PLOT;
    let mut shapes = Vec::new();
    for i in 0..4 {
        let y = h * f64::from(i) / 4.0;
        shapes.push(k.line(p, "Grid line", &format!("M0 {y} H{w}"), BORDER, 1.0)?);
    }
    shapes.push(k.line(p, "Baseline", &format!("M0 {h} H{w}"), MUTED, 1.0)?);
    let area = format!("{} V{h} H0 Z", polyline(&REVENUE, PLOT));
    shapes.push(k.shape(p, "Area", &area, json!({"fill": PRIMARY, "opacity": 0.12}))?);
    let last = k.line(p, "Last year", &polyline(&LAST_YEAR, PLOT), SEA, 2.0)?;
    k.set(last, json!({"dashes": [6, 6]}))?;
    shapes.push(last);
    shapes.push(k.line(p, "This year", &polyline(&REVENUE, PLOT), PRIMARY, 3.0)?);
    fit(k, p, &shapes)?;
    axis(k, card, &["1 Mar", "15 Mar", "29 Mar", "12 Apr", "26 Apr", "10 May", "24 May"])
}

fn kinds(k: &mut Kit, grid: NodeId) -> Result<()> {
    let card = card(k, grid, "Bookings by kind", 1, "")?;
    let ring =
        k.stack(card, "Ring", H, 0.0, json!(0), json!({"sizingH": "fill", "sizingV": "fill", "mainAlign": "center", "crossAlign": "center"}))?;
    let donut = k.frame(ring, "Donut", [0.0, 0.0, 132.0, 132.0], json!({"fills": [], "clip": false}))?;
    let mut start = -90.0;
    for (label, share, color) in KINDS {
        let sweep = share * 360.0;
        k.ellipse(donut, label, [0.0, 0.0, 132.0, 132.0], json!({"fill": color, "arcStart": start, "arcSweep": sweep - 2.0, "arcInner": 0.68}))?;
        start += sweep;
    }
    let legend = k.stack(card, "Legend", V, 8.0, json!(0), json!({"sizingH": "fill"}))?;
    for (label, share, color) in KINDS {
        let row = k.stack(legend, label, H, 8.0, json!(0), json!({"sizingH": "fill", "crossAlign": "center"}))?;
        k.ellipse(row, "Dot", [0.0, 0.0, 10.0, 10.0], json!({"fill": color}))?;
        k.text_with(row, label, "Body/S", INK, json!({"sizingH": "fill"}))?;
        k.text(row, &format!("{:.0}%", share * 100.0), "Label/S", MUTED)?;
    }
    Ok(())
}

fn weekdays(k: &mut Kit, grid: NodeId) -> Result<()> {
    let card = card(k, grid, "Busiest days", 1, "Per weekday")?;
    k.bar_chart(card, [230.0, 170.0], &WEEKDAYS, "Label/S")
}

fn bookings(k: &mut Kit, grid: NodeId) -> Result<()> {
    let card = card(k, grid, "Latest bookings", 3, "See all")?;
    let table = k.stack(card, "Table", V, 0.0, json!(0), json!({"sizingH": "fill"}))?;
    let header = ["Walk", "Guide", "Date", "Status", "Amount"];
    for (i, cells) in std::iter::once(header).chain(BOOKINGS).enumerate() {
        let edge = if i < BOOKINGS.len() { json!({"stroke": BORDER, "strokeSides": BOTTOM_EDGE}) } else { json!({}) };
        let row = k.stack(
            table,
            if i == 0 { "Header" } else { "Row" },
            H,
            12.0,
            json!([10, 0, 10, 0]),
            merged(json!({"sizingH": "fill", "crossAlign": "center"}), edge),
        )?;
        for (column, cell) in cells.into_iter().enumerate() {
            cell_in(k, row, column, cell, i == 0)?;
        }
    }
    Ok(())
}

/// One table cell: the first column is the widest, the status is a pill, the amount is right-aligned.
fn cell_in(k: &mut Kit, row: NodeId, column: usize, cell: &str, header: bool) -> Result<()> {
    let (style, color) = if header {
        ("Label/S", MUTED)
    } else if column == 0 {
        ("Label/M", INK)
    } else {
        ("Body/S", INK)
    };
    let width: Value = match column {
        0 => json!({"sizingH": "fill"}),
        4 => json!({"sizingH": "fixed", "width": 80, "textAlign": "right", "textAutoResize": "height"}),
        _ => json!({"sizingH": "fixed", "width": 140, "textAutoResize": "height"}),
    };
    if column == 3 && !header {
        let holder = k.stack(row, "Status", H, 0.0, json!(0), json!({"sizingH": "fixed", "width": 140}))?;
        let [color, fill] = if cell == "Waiting" { ["#8A5A0B", "#FCEFD9"] } else { [PRIMARY, SOFT] };
        return k.chip(holder, cell, ["Label/S", color], fill, [3.0, 10.0], 999.0).map(drop);
    }
    k.text_with(row, cell, style, color, width).map(drop)
}
