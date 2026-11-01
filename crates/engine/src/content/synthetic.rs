//! Synthetic large documents for benchmarks (`figcraft-cli perf`) and scale tests.
//!
//! A page of "screens" (top-level frames) laid out in a grid. Each screen holds an auto layout
//! header (text and icons), a card group, three instances of a shared button component and a grid
//! of rectangles, ellipses and text, so every layer kind the hot paths care about is present.

use figcraft_color::{Color, Paint};
use figcraft_doc::{AutoLayout, DocResult, Document, NodeId, NodeKind};

/// Screen size and spacing (page units).
pub const SCREEN_W: f64 = 360.0;
pub const SCREEN_H: f64 = 240.0;
pub const SCREEN_GAP: f64 = 80.0;
/// Layers per screen (the last screen may hold fewer).
pub const PER_SCREEN: usize = 50;

fn text(content: &str) -> NodeKind {
    NodeKind::Text(figcraft_doc::TextData::new(content))
}

fn ellipse() -> NodeKind {
    NodeKind::Ellipse { start: 0.0, sweep: std::f64::consts::TAU, inner: 0.0 }
}

fn fill(doc: &mut Document, id: NodeId, c: Color) {
    if let Some(n) = doc.node_mut(id) {
        n.fills = vec![Paint::solid(c)];
    }
}

fn add_screen_header(doc: &mut Document, screen: NodeId, s: usize) -> DocResult<()> {
    let header = doc.create(screen, NodeKind::Frame, "Header", 0.0, 0.0, SCREEN_W, 40.0)?;
    if let Some(n) = doc.node_mut(header) {
        n.layout = Some(AutoLayout { gap: 8.0, padding: [8.0; 4], ..AutoLayout::default() });
        n.fills = vec![Paint::solid(Color::from_rgb8_const(0xF5, 0xF5, 0xF5))];
    }
    doc.create(header, text(&format!("Screen {}", s + 1)), "Title", 0.0, 0.0, 120.0, 16.0)?;
    for k in 0..2 {
        let icon = doc.create(header, ellipse(), &format!("Icon {}", k + 1), 0.0, 0.0, 20.0, 20.0)?;
        fill(doc, icon, Color::from_rgb8_const(0x99, 0x99, 0x99));
    }
    Ok(())
}

fn add_card_group(doc: &mut Document, screen: NodeId) -> DocResult<()> {
    let card = doc.create(screen, NodeKind::Group, "Card", 12.0, 52.0, 160.0, 64.0)?;
    // Children are in the group's space; normalising fits the group around them.
    let cbg = doc.create(card, NodeKind::Rectangle, "Card background", 0.0, 0.0, 160.0, 64.0)?;
    fill(doc, cbg, Color::from_rgb8_const(0xEE, 0xF4, 0xFF));
    doc.create(card, ellipse(), "Avatar", 8.0, 8.0, 32.0, 32.0)?;
    doc.create(card, text("Ada Lovelace"), "Name", 48.0, 8.0, 100.0, 16.0)?;
    doc.create(card, text("Analyst"), "Role", 48.0, 28.0, 100.0, 16.0)?;
    Ok(())
}

fn add_screen_grid(doc: &mut Document, screen: NodeId, s: usize, rest: usize, made: &mut usize) -> DocResult<()> {
    for k in 0..rest {
        let (gx, gy) = (184.0 + (k % 8) as f64 * 21.0, 52.0 + (k / 8) as f64 * 34.0);
        let hue = ((s * PER_SCREEN + k) % 360) as f32 / 360.0;
        // The grid starts with a rectangle so even the smallest workload has one.
        let kind = (k + 2) % 5;
        let id = match kind {
            0 => doc.create(screen, text(&format!("Item {k}")), "Text", gx, gy, 20.0, 14.0)?,
            1 | 3 => doc.create(screen, ellipse(), "Ellipse", gx, gy, 18.0, 18.0)?,
            _ => doc.create(screen, NodeKind::Rectangle, "Rectangle", gx, gy, 18.0, 18.0)?,
        };
        if kind != 0 {
            fill(doc, id, Color::new(hue, 0.5, 1.0 - hue, 1.0));
            if let Some(n) = doc.node_mut(id) {
                n.corners.radii = [3.0; 4];
            }
        }
        *made += 1;
    }
    Ok(())
}

/// A page of about `layers` layers (pages not counted), normalised like an opened file. Returns
/// the document and its page.
pub fn large_document(layers: usize) -> DocResult<(Document, NodeId)> {
    let mut doc = Document::new("Synthetic");
    let page = doc.first_page().ok_or(figcraft_doc::DocError::Invalid("new document has no page".into()))?;
    let layers = layers.clamp(10, 2_000_000);
    // The shared button component (3 layers), above the screens.
    let button = doc.create(
        page,
        NodeKind::Component { description: String::new(), props: Default::default(), prop_order: Vec::new() },
        "Button",
        0.0,
        -120.0,
        96.0,
        32.0,
    )?;
    let bg = doc.create(button, NodeKind::Rectangle, "Background", 0.0, 0.0, 96.0, 32.0)?;
    fill(&mut doc, bg, Color::from_rgb8_const(0x0D, 0x99, 0xFF));
    if let Some(n) = doc.node_mut(bg) {
        n.corners.radii = [6.0; 4];
    }
    doc.create(button, text("Button"), "Label", 12.0, 8.0, 72.0, 16.0)?;
    let screens = (layers.saturating_sub(3)).div_ceil(PER_SCREEN).max(1);
    let cols = ((screens as f64).sqrt().ceil() as usize).max(1);
    let mut made = 3usize;
    for s in 0..screens {
        let (x, y) = ((s % cols) as f64 * (SCREEN_W + SCREEN_GAP), (s / cols) as f64 * (SCREEN_H + SCREEN_GAP));
        let screen = doc.create(page, NodeKind::Frame, &format!("Screen {}", s + 1), x, y, SCREEN_W, SCREEN_H)?;
        made += 1;
        let budget = |made: usize| layers.saturating_sub(made);
        // Auto layout header: title + two icons (4 layers).
        if budget(made) >= 4 {
            add_screen_header(&mut doc, screen, s)?;
            made += 4;
        }
        // A card group (5 layers).
        // The card and the instances leave one layer for the grid, which starts with a rectangle.
        if budget(made) > 5 {
            add_card_group(&mut doc, screen)?;
            made += 5;
        }
        // Three button instances (3 layers each).
        for k in 0..3 {
            if budget(made) < 3 + 1 {
                break;
            }
            crate::components::create_instance(&mut doc, button, screen, 12.0 + k as f64 * 108.0, SCREEN_H - 44.0)?;
            made += 3;
        }
        // Fill the rest of the screen with a grid of shapes and text.
        let rest = PER_SCREEN.saturating_sub(4 + 5 + 9 + 1).min(budget(made));
        add_screen_grid(&mut doc, screen, s, rest, &mut made)?;
        if budget(made) == 0 {
            break;
        }
    }
    // Settle derived state (text sizes, group bounds, auto layout) as after opening a file.
    let empty = Document::new("");
    crate::normalize(&mut doc, &empty);
    Ok((doc, page))
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_about_the_requested_size() {
        for n in [10, 100, 1000, 2500] {
            let (doc, page) = super::large_document(n).unwrap();
            let layers = doc.node_count() - 1;
            assert!(layers >= n && layers <= n + 3, "{n} → {layers}");
            assert!(doc.validate().is_empty());
            assert!(doc.page_content_bounds(page).is_some());
        }
    }

    #[test]
    fn small_documents_hold_the_benchmark_targets() {
        for n in [10, 11, 12, 13, 14, 15] {
            let (doc, page) = super::large_document(n).unwrap();
            let nodes: Vec<_> = doc.subtree(page).into_iter().filter_map(|id| doc.node(id)).collect();
            let has = |f: &dyn Fn(&figcraft_doc::Node) -> bool| nodes.iter().any(|node| f(node));
            assert!(has(&|x| matches!(x.kind, figcraft_doc::NodeKind::Rectangle) && x.name == "Rectangle"), "{n}: no rectangle");
            assert!(has(&|x| x.name == "Title"), "{n}: no title");
            assert!(has(&|x| matches!(x.kind, figcraft_doc::NodeKind::Component { .. })), "{n}: no component");
        }
    }
}
