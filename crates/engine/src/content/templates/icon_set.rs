//! Icon set starter: FigCraft's own template icons as components on a 24 px grid (2 px round
//! strokes in one colour style), an anatomy board that shows the grid, the margin and the keyline
//! shapes at twenty times the size, and a preview of one icon at four sizes.

use figcraft_doc::NodeId;
use serde_json::json;

use super::glyphs;
use super::kit::{BORDER, CARD, H, ICON, INK, Kit, MUTED, PAGE, V, merged};
use crate::Result;

const STROKE: &str = "Icon/Stroke";
/// The anatomy board draws one grid unit this large.
const UNIT: f64 = 20.0;
const GUIDE: &str = "#E2574C";
/// The width of the icon boards: six 88 px cells, their gaps, the padding and a little slack.
const LIBRARY: f64 = 6.0 * 88.0 + 5.0 * 16.0 + 72.0;

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.colors(&[(STROKE, "#14201C")])?;
    k.rename_page("Icons")?;
    let page = k.page()?;
    anatomy(k, page)?;
    // The icons and their preview sit in a column beside the anatomy board.
    let column = k.stack(page, "Library", V, 32.0, json!(0), json!({"x": ICON * UNIT + 64.0 + 80.0}))?;
    let home = icons(k, column)?;
    sizes(k, column, home)
}

/// A titled white panel.
fn board(k: &mut Kit, parent: NodeId, title: &str, note: &str, width: f64) -> Result<NodeId> {
    let props = json!({"fill": CARD, "stroke": BORDER, "strokeWeight": 1, "cornerRadius": 24, "sizingH": "fixed", "width": width});
    let board = k.stack(parent, title, V, 24.0, json!(32), props)?;
    let head = k.stack(board, "Heading", V, 4.0, json!(0), json!({"sizingH": "fill"}))?;
    k.text(head, title, "Heading/M", INK)?;
    k.para(head, note, "Body/S", MUTED)?;
    Ok(board)
}

/// The grid an icon is drawn on, enlarged: every unit, the 2 px margin, and the circle and square
/// keylines that keep icons the same optical size, around one icon.
fn anatomy(k: &mut Kit, page: NodeId) -> Result<()> {
    let side = ICON * UNIT;
    let board = board(k, page, "Anatomy", "24 px grid · 2 px margin · circle and square keylines · 2 px round strokes", side + 64.0)?;
    let grid = k.frame(board, "Grid", [0.0, 0.0, side, side], json!({"fill": PAGE, "clip": true}))?;
    let lines: String = (1..24).map(|i| f64::from(i) * UNIT).map(|at| format!("M{at} 0 V{side} M0 {at} H{side} ")).collect();
    k.line(grid, "Units", lines.trim_end(), BORDER, 1.0)?;
    let dashed = json!({"fills": [], "stroke": GUIDE, "strokeWeight": 1.5, "dashes": [6, 6]});
    let (margin, live) = (2.0 * UNIT, side - 4.0 * UNIT);
    k.rect(grid, "Margin", [margin, margin, live, live], dashed.clone())?;
    k.ellipse(grid, "Circle keyline", [margin, margin, live, live], dashed.clone())?;
    k.rect(grid, "Square keyline", [3.0 * UNIT, 3.0 * UNIT, side - 6.0 * UNIT, side - 6.0 * UNIT], merged(dashed, json!({"cornerRadius": UNIT})))?;
    // The enlarged icon's stroke is enlarged with it.
    let (_, stroke) = k.icon_of("frame", grid, &glyphs::HOME, [0.0, 0.0], side, STROKE)?;
    k.set(stroke, json!({"strokeWeight": 2.0 * UNIT, "opacity": 0.9}))
}

/// Every icon as a component with its name under it. Returns the first one.
fn icons(k: &mut Kit, parent: NodeId) -> Result<NodeId> {
    let board = board(k, parent, "Icons", "Components: drag one from the Assets panel, or copy one and redraw it on the grid", LIBRARY)?;
    let grid = k.stack(board, "Set", H, 16.0, json!(0), json!({"sizingH": "fill", "wrap": true, "crossGap": 24}))?;
    let mut first = None;
    for glyph in glyphs::ALL {
        let cell = json!({"sizingH": "fixed", "width": 88, "crossAlign": "center"});
        let cell = k.stack(grid, glyph.name, V, 10.0, json!(0), cell)?;
        let (icon, _) = k.icon_of("component", cell, glyph, [0.0, 0.0], ICON, STROKE)?;
        k.set(icon, json!({"name": format!("Icon/{}", glyph.name)}))?;
        k.text(cell, glyph.name, "Label/S", MUTED)?;
        first.get_or_insert(icon);
    }
    first.ok_or_else(|| crate::EngineError::Other("the icon set is empty".into()))
}

/// One icon at four sizes: instances scale, and their strokes keep their weight.
fn sizes(k: &mut Kit, parent: NodeId, icon: NodeId) -> Result<()> {
    let board = board(k, parent, "Sizes", "Instances at 16, 24, 32 and 48 px", LIBRARY)?;
    let row = k.stack(board, "Row", H, 40.0, json!(0), json!({"crossAlign": "max"}))?;
    for size in [16.0, 24.0, 32.0, 48.0] {
        let cell = k.stack(row, &format!("{size} px"), V, 10.0, json!(0), json!({"crossAlign": "center"}))?;
        k.instance(icon, cell, json!({}), json!({"width": size, "height": size}))?;
        k.text(cell, &format!("{size}"), "Label/S", MUTED)?;
    }
    Ok(())
}
