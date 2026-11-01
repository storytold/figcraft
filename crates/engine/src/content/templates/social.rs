//! Social post sizes: one invented announcement ("Summer trail series") laid out in five common
//! sizes by the same code. Wide sizes put the words beside the picture, tall ones above it; type
//! and spacing scale with the frame.

use figcraft_doc::NodeId;
use serde_json::json;

use super::glyphs;
use super::kit::{ACCENT, DEEP, H, INK, Kit, ON_BRAND, V};
use crate::Result;

/// (name, width, height, x, y) on the canvas.
const SIZES: [(&str, f64, f64, f64, f64); 5] = [
    ("Square", 1080.0, 1080.0, 0.0, 0.0),
    ("Portrait", 1080.0, 1350.0, 1200.0, 0.0),
    ("Story", 1080.0, 1920.0, 2400.0, 0.0),
    ("Link preview", 1200.0, 630.0, 3600.0, 0.0),
    ("Banner", 1500.0, 500.0, 3600.0, 750.0),
];

pub(super) fn build(k: &mut Kit) -> Result<()> {
    k.foundations()?;
    k.rename_page("Posts")?;
    let page = k.page()?;
    for (name, w, h, x, y) in SIZES {
        post(k, page, &format!("{name} · {w} × {h}"), [w, h], [x, y])?;
    }
    Ok(())
}

fn post(k: &mut Kit, page: NodeId, name: &str, [w, h]: [f64; 2], [x, y]: [f64; 2]) -> Result<()> {
    let wide = w > h * 1.5;
    // Type and spacing follow the shorter side.
    let u = (w.min(h) / 18.0).round();
    let frame = json!({"fill": DEEP, "clip": true, "sizingH": "fixed", "sizingV": "fixed", "width": w, "height": h, "x": x, "y": y});
    let root = k.stack(page, name, if wide { H } else { V }, 0.0, json!(0), frame)?;
    let words = k.stack(root, "Words", V, u, json!(u * 1.2), json!({"sizingH": "fill", "sizingV": "fill", "mainAlign": "spaceBetween"}))?;
    let logo = k.stack(words, "Logo", H, u * 0.3, json!(0), json!({"crossAlign": "center"}))?;
    k.icon(logo, &glyphs::MOUNTAIN, (u * 0.9).round(), ACCENT)?;
    k.sized_text(logo, "Ridgeline Trails", [u * 0.62, u * 0.9], "Semi Bold", ON_BRAND, json!({}))?;
    let message = k.stack(words, "Message", V, u * 0.5, json!(0), json!({"sizingH": "fill"}))?;
    k.sized_text(message, "SUMMER TRAIL SERIES", [u * 0.5, u * 0.7], "Semi Bold", ACCENT, json!({"letterSpacing": "8%"}))?;
    let wrap = json!({"sizingH": "fill", "textAutoResize": "height"});
    let headline = u * 1.8;
    k.sized_text(message, "Six Saturdays.\nSix summits.", [headline, headline * 1.08], "Bold", ON_BRAND, wrap.clone())?;
    k.sized_text(message, "Guided walks for every pace, starting 14 June.", [u * 0.6, u * 0.9], "Regular", "#CFE6DE", wrap)?;
    let action = k.stack(words, "Action", H, u * 0.6, json!(0), json!({"crossAlign": "center"}))?;
    let button = k.stack(action, "Button", H, 0.0, json!([u * 0.4, u * 0.8, u * 0.4, u * 0.8]), json!({"fill": ACCENT, "cornerRadius": 999}))?;
    k.sized_text(button, "Join the first walk", [u * 0.55, u * 0.8], "Semi Bold", INK, json!({}))?;
    k.sized_text(action, "ridgeline.example", [u * 0.5, u * 0.8], "Medium", "#CFE6DE", json!({}))?;
    // The picture takes the rest: the right-hand side of a wide post, the bottom of a tall one.
    let (size, fill) = if wide {
        ([w * 0.4, h], json!({"sizingV": "fill"}))
    } else {
        ([w, (h * if h > w { 0.36 } else { 0.3 }).round()], json!({"sizingH": "fill"}))
    };
    k.scene(root, "Picture", size, fill).map(drop)
}
