//! View menu, preferences and tools. View state is per document and drivable like everything else.

use figcraft_doc::*;
use serde_json::{Value, json};

use super::{CommandSpec, ok};
use crate::params::*;
use crate::tools::ToolId;
use crate::{EngineError, Result, Session, ViewState, bad, spec};

fn view(s: &mut Session) -> Result<&mut ViewState> {
    Ok(&mut s.ds_mut()?.view)
}

fn flag(s: &mut Session, p: &Value, f: fn(&mut ViewState) -> &mut bool) -> Result<Value> {
    let v = view(s)?;
    let slot = f(v);
    *slot = boolean(p, "on").unwrap_or(!*slot);
    Ok(json!({"on": *slot}))
}

/// One of a view's on/off settings.
type Slot = fn(&mut ViewState) -> &mut bool;

/// The View menu's on/off toggles (Pixel preview, with its scale, has its own command), and
/// whether the preferences keep each: files opened or made later start with the kept ones. Outline
/// modes and the sRGB soft-proof stay with their file, so a new file never opens in one.
const TOGGLES: &[(&str, Slot, bool)] = &[
    ("view.pixelGrid", |v| &mut v.pixel_grid, true),
    ("view.layoutGrids", |v| &mut v.layout_grids, true),
    ("view.rulers", |v| &mut v.rulers, true),
    ("view.slices", |v| &mut v.show_slices, true),
    ("view.comments", |v| &mut v.comments, true),
    ("view.multiplayerCursors", |v| &mut v.multiplayer_cursors, true),
    ("view.outlines", |v| &mut v.outlines, false),
    ("view.outlinesSelection", |v| &mut v.outlines_selection, false),
    ("view.outlinesHiddenLayers", |v| &mut v.outline_hidden, true),
    ("view.outlinesObjectBounds", |v| &mut v.outline_bounds, true),
    ("view.maskOutlines", |v| &mut v.mask_outlines, true),
    ("view.frameOutlines", |v| &mut v.frame_outlines, true),
    ("view.proofSrgb", |v| &mut v.proof_srgb, false),
    ("view.snapGeometry", |v| &mut v.snap_geometry, true),
    ("view.snapObjects", |v| &mut v.snap_objects, true),
    ("view.snapPixel", |v| &mut v.snap_pixel, true),
];

/// View toggle `id`: the file's view changes, and the preferences remember a kept one.
fn toggle(s: &mut Session, p: &Value, id: &str) -> Result<Value> {
    let Some(&(_, f, keep)) = TOGGLES.iter().find(|(k, ..)| *k == id) else { return Err(bad(id, "not a View toggle")) };
    let r = flag(s, p, f)?;
    if keep && let Some(on) = r.get("on").and_then(Value::as_bool) {
        s.prefs.view.insert(id.to_string(), on);
    }
    Ok(r)
}

/// Set `v`'s View toggles to the ones the preferences keep (the others stay as they are).
pub(crate) fn apply_kept(prefs: &crate::Prefs, v: &mut ViewState) {
    for (id, f, _) in TOGGLES.iter().filter(|t| t.2) {
        if let Some(on) = prefs.view.get(*id) {
            *f(v) = *on;
        }
    }
}

/// Whether the View menu's toggle `id` is on (the menus show a check mark); `None` for a command
/// that isn't one.
pub fn checked(s: &Session, id: &str) -> Option<bool> {
    let mut v = s.ds().ok()?.view.clone();
    if id == "view.pixelPreview" {
        return Some(v.pixel_preview);
    }
    TOGGLES.iter().find(|(k, ..)| *k == id).map(|(_, f, _)| *f(&mut v))
}

/// Pixel preview: toggles, or turns on at a resolution of 1x or 2x.
fn pixel_preview(s: &mut Session, p: &Value) -> Result<Value> {
    let scale = match p.get("scale") {
        None | Some(Value::Null) => None,
        Some(v) => match v.as_u64() {
            Some(k @ (1 | 2)) => Some(k as u8),
            _ => return Err(bad("view.pixelPreview", "`scale` must be 1 or 2")),
        },
    };
    let v = view(s)?;
    if let Some(k) = scale {
        v.pixel_scale = k;
    }
    v.pixel_preview = boolean(p, "on").unwrap_or(scale.is_some() || !v.pixel_preview);
    Ok(json!({"on": v.pixel_preview, "scale": v.pixel_scale}))
}

fn zoom_to(s: &mut Session, z: f64) -> Result<Value> {
    let v = view(s)?;
    let c = (v.cx, v.cy);
    v.set_zoom_at(z, c);
    Ok(json!({"zoom": v.zoom}))
}

/// How far an arrow key pans the view with nothing selected (screen pixels; Shift: ten times).
pub const PAN_STEP_PX: f64 = 20.0;

/// `view.pan`: the arrow keys with nothing selected move the view by keyboard steps.
fn pan(s: &mut Session, p: &Value) -> Result<Value> {
    let step = if boolean(p, "big").unwrap_or(false) { PAN_STEP_PX * 10.0 } else { PAN_STEP_PX };
    let dir = |k| num(p, k).filter(|v| v.is_finite()).unwrap_or(0.0).clamp(-100.0, 100.0) * step;
    let (dx, dy) = (dir("dx"), dir("dy"));
    let v = view(s)?;
    // Scrolling moves the content; panning right shows what is to the right.
    v.scroll(-dx, -dy, false);
    Ok(json!({"cx": v.cx, "cy": v.cy}))
}

/// Figma's zoom steps.
const STEPS: &[f64] = &[0.02, 0.03, 0.06, 0.13, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0];

fn step(z: f64, up: bool) -> f64 {
    if up {
        STEPS.iter().copied().find(|s| *s > z * 1.001).unwrap_or(256.0)
    } else {
        STEPS.iter().rev().copied().find(|s| *s < z * 0.999).unwrap_or(0.02)
    }
}

/// The zoom tool: a click zooms a step in (or out) about the clicked point, a dragged area fills
/// the view.
fn zoom_at(s: &mut Session, p: &Value) -> Result<Value> {
    let (Some(x), Some(y)) = (num(p, "x").filter(|v| v.is_finite()), num(p, "y").filter(|v| v.is_finite())) else {
        return Err(bad("view.zoomAt", "`x` and `y` (page coordinates) are required"));
    };
    let v = view(s)?;
    match (num(p, "width"), num(p, "height")) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => {
            v.fit(figcraft_geom::Rect::new(x, y, x + w, y + h), ViewState::MAX_ZOOM);
        }
        _ => {
            let z = step(v.zoom, !boolean(p, "out").unwrap_or(false));
            v.set_zoom_at(z, (x, y));
        }
    }
    Ok(json!({"zoom": v.zoom, "cx": v.cx, "cy": v.cy}))
}

fn zoom_frame(s: &mut Session, dir: i64) -> Result<Value> {
    let ds = s.ds()?;
    let frames = ds.doc.top_frames(ds.page);
    if frames.is_empty() {
        return ok();
    }
    let cur = ds.selection.first().and_then(|i| ds.doc.top_level_frame(*i)).and_then(|f| frames.iter().position(|x| *x == f));
    let n = frames.len() as i64;
    let next = match cur {
        Some(c) => (c as i64 + dir).rem_euclid(n) as usize,
        None => 0,
    };
    let Some(f) = frames.get(next).copied() else { return ok() };
    let b = ds.doc.world_bounds(f).unwrap_or_default();
    let ds = s.ds_mut()?;
    ds.selection = vec![f];
    ds.view.fit(b, 256.0);
    Ok(json!({"frame": f}))
}

fn page_step(s: &mut Session, dir: i64) -> Result<Value> {
    let ds = s.ds()?;
    let pages = &ds.doc.pages;
    let i = pages.iter().position(|p| *p == ds.page).unwrap_or(0) as i64;
    let n = pages.len() as i64;
    // Dividers never open: step over them, and stay put when only dividers lie ahead.
    let mut next = i + dir;
    let pg = loop {
        let page = usize::try_from(next).ok().and_then(|k| pages.get(k).copied());
        match page {
            Some(p) if !ds.doc.is_divider(p) => break p,
            Some(_) if dir != 0 && next >= 0 && next < n => next += dir,
            _ => break ds.page,
        }
    };
    let ds = s.ds_mut()?;
    ds.page = pg;
    ds.selection.clear();
    ds.zoom_to_fit();
    Ok(json!({"page": pg}))
}

fn ui(s: &mut Session, kind: &str) -> Result<Value> {
    s.request_ui(kind, json!({}));
    ok()
}

/// Move the keyboard `step` regions on (toolbar, layers, canvas, properties): the UI does it.
fn region(s: &mut Session, step: i64) -> Result<Value> {
    s.request_ui("focus.region", json!({"step": step}));
    ok()
}

fn pref(s: &mut Session, p: &Value, f: fn(&mut crate::Prefs) -> &mut bool) -> Result<Value> {
    let slot = f(&mut s.prefs);
    *slot = boolean(p, "on").unwrap_or(!*slot);
    Ok(json!({"on": *slot}))
}

pub(crate) fn tool(s: &mut Session, t: ToolId) -> Result<Value> {
    // A file you can't edit now (view only, or on its way to a server): moving around only.
    if let Some(why) = s.active_read_only().filter(|_| !matches!(t, ToolId::Move | ToolId::Hand)) {
        return Err(EngineError::Disabled(format!("tool.{}", t.id()), why.into()));
    }
    s.set_tool(t);
    Ok(json!({"tool": t.id()}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!("view.zoomIn", "Zoom in", "{}", false, |s, _| {
            let z = view(s)?.zoom;
            zoom_to(s, step(z, true))
        }),
        spec!("view.zoomOut", "Zoom out", "{}", false, |s, _| {
            let z = view(s)?.zoom;
            zoom_to(s, step(z, false))
        }),
        spec!("view.zoom100", "Zoom to 100%", "{}", false, |s, _| zoom_to(s, 1.0)),
        spec!("view.zoom50", "Zoom to 50%", "{}", false, |s, _| zoom_to(s, 0.5)),
        spec!("view.zoom200", "Zoom to 200%", "{}", false, |s, _| zoom_to(s, 2.0)),
        spec!("view.zoomFit", "Zoom to fit", "{}", false, |s, _| {
            let ds = s.ds_mut()?;
            if let Some(b) = ds.doc.page_content_bounds(ds.page) {
                ds.view.fit(b, 256.0);
            }
            Ok(json!({"zoom": ds.view.zoom}))
        }),
        spec!("view.zoomSelection", "Zoom to selection", "{}", false, |s, _| {
            let ds = s.ds_mut()?;
            if let Some(b) = ds.doc.bounds_of(&ds.selection) {
                ds.view.fit(b, 256.0);
            }
            Ok(json!({"zoom": ds.view.zoom}))
        }),
        spec!("view.set", "Set view", "{zoom?, cx?, cy? (page coordinate at the centre), vw?, vh? (viewport size)}", false, |s, p| {
            let v = view(s)?;
            if let Some(z) = num(p, "zoom") {
                v.zoom = z.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM);
            }
            v.cx = num_or(p, "cx", v.cx);
            v.cy = num_or(p, "cy", v.cy);
            if let Some(w) = num(p, "vw") {
                v.vw = w.clamp(10.0, 20000.0);
            }
            if let Some(h) = num(p, "vh") {
                v.vh = h.clamp(10.0, 20000.0);
            }
            Ok(serde_json::to_value(&*v).unwrap_or(Value::Null))
        }),
        spec!(
            "view.zoomAt",
            "Zoom tool",
            "{x, y (page point kept in place), out? (zoom out a step), width?, height? (an area to fit instead)}",
            false,
            zoom_at
        ),
        spec!("view.get", "Get view", "{}", false, |s, _| Ok(serde_json::to_value(&s.ds()?.view).unwrap_or(Value::Null))),
        spec!("view.prevFrame", "Zoom to previous frame", "{}", false, |s, _| zoom_frame(s, -1)),
        spec!("view.nextFrame", "Zoom to next frame", "{}", false, |s, _| zoom_frame(s, 1)),
        spec!("view.prevPage", "Previous page", "{}", false, |s, _| page_step(s, -1)),
        spec!("view.nextPage", "Next page", "{}", false, |s, _| page_step(s, 1)),
        spec!("view.pixelGrid", "Pixel grid", "{on?}", false, |s, p| toggle(s, p, "view.pixelGrid")),
        spec!("view.layoutGrids", "Layout grids", "{on?}", false, |s, p| toggle(s, p, "view.layoutGrids")),
        spec!("view.rulers", "Rulers", "{on?}", false, |s, p| toggle(s, p, "view.rulers")),
        spec!("view.slices", "Slices", "{on?}", false, |s, p| toggle(s, p, "view.slices")),
        spec!("view.comments", "Comments", "{on?}", false, |s, p| toggle(s, p, "view.comments")),
        spec!("view.multiplayerCursors", "Multiplayer cursors", "{on?}  others' cursors in a live session", false, |s, p| {
            toggle(s, p, "view.multiplayerCursors")
        }),
        spec!("view.pan", "Pan the view", "{dx, dy (arrow-key steps, e.g. 1 or -1), big? (ten times as far, like Shift)}", false, pan),
        spec!("view.outlines", "Outlines", "{on?}", false, |s, p| toggle(s, p, "view.outlines")),
        spec!("view.outlinesSelection", "Outlines for selection", "{on?}  outline mode for the selected layers only", false, |s, p| {
            toggle(s, p, "view.outlinesSelection")
        }),
        spec!(
            "view.outlinesHiddenLayers",
            "Include hidden layers in outlines",
            "{on?}  outline mode (and outlines for the selection) outlines hidden layers too; they can be clicked there",
            false,
            |s, p| { toggle(s, p, "view.outlinesHiddenLayers") }
        ),
        spec!(
            "view.outlinesObjectBounds",
            "Include object bounds in outlines",
            "{on?}  outline mode also draws each layer's box (groups, text, vectors); a click on a box's edge picks its layer",
            false,
            |s, p| { toggle(s, p, "view.outlinesObjectBounds") }
        ),
        spec!("view.maskOutlines", "Mask outlines", "{on?}", false, |s, p| toggle(s, p, "view.maskOutlines")),
        spec!("view.pixelPreview", "Pixel preview", "{on?, scale? (1|2: rendered pixels per page pixel; turns it on)}", false, pixel_preview),
        spec!("view.frameOutlines", "Frame outlines", "{on?}", false, |s, p| toggle(s, p, "view.frameOutlines")),
        spec!(
            "view.proofSrgb",
            "Soft-proof sRGB",
            "{on?}  mark the colours of a Display P3 file that sRGB screens and files can't show",
            false,
            |s, p| { toggle(s, p, "view.proofSrgb") }
        ),
        spec!("view.snapGeometry", "Snap to geometry", "{on?}", false, |s, p| toggle(s, p, "view.snapGeometry")),
        spec!("view.snapObjects", "Snap to objects", "{on?}", false, |s, p| toggle(s, p, "view.snapObjects")),
        spec!("view.snapPixel", "Snap to pixel grid", "{on?}", false, |s, p| toggle(s, p, "view.snapPixel")),
        spec!("view.toggleUi", "Show/Hide UI", "{on?}", false, |s, p| flag(s, p, |v| &mut v.ui_hidden)),
        spec!("view.fullscreen", "Full screen", "{on?}  the window fills the screen (the desktop app; a no-op headless)", false, |s, p| {
            s.request_ui("window.fullscreen", json!({"on": boolean(p, "on")}));
            ok()
        }),
        spec!("view.layers", "Layers panel", "{}", false, |s, _| ui(s, "panel.layers")),
        spec!("view.assets", "Assets panel", "{}", false, |s, _| ui(s, "panel.assets")),
        spec!("view.design", "Design panel", "{}", false, |s, _| ui(s, "panel.design")),
        spec!("view.prototype", "Prototype panel", "{}", false, |s, _| ui(s, "panel.prototype")),
        spec!("view.designPrototype", "Switch between Design and Prototype", "{}", false, |s, _| ui(s, "panel.designPrototype")),
        spec!("view.devMode", "Dev Mode", "{}", false, |s, _| ui(s, "panel.devMode")),
        spec!("view.leftSidebar", "Show/Hide left sidebar", "{}", false, |s, _| ui(s, "panel.leftSidebar")),
        spec!(
            "view.panelWidths",
            "Reset panel widths",
            "{left?, right? (the panels' widths in pixels, kept within their limits)}  without either, both go back to their default width (a double-click on a panel's edge does it for that panel)",
            false,
            |s, p| {
                let width = |k| num(p, k).filter(|w| w.is_finite()).map(|w| w.clamp(0.0, 10_000.0));
                s.request_ui("panel.widths", json!({"left": width("left"), "right": width("right")}));
                ok()
            }
        ),
        // Several windows: the desktop app opens and focuses them (no-ops headless and in the browser).
        spec!(
            "window.new",
            "New window",
            "{path?, index?}  a new window with a new file, the file at `path`, or open file `index` moved into it",
            false,
            |s, p| {
                let index = if num(p, "index").is_some() { Some(req_index("window.new", p, "index")?) } else { None };
                s.request_ui("window.new", json!({"path": string(p, "path"), "index": index}));
                ok()
            }
        ),
        spec!("window.focus", "Bring window to front", "{id}  a window id (control channel `window.list`)", false, |s, p| {
            let id = req_index("window.focus", p, "id")?;
            s.request_ui("window.focus", json!({"id": id}));
            ok()
        }),
        spec!("view.nextRegion", "Focus next region", "{}", false, |s, _| region(s, 1)),
        spec!("view.previousRegion", "Focus previous region", "{}", false, |s, _| region(s, -1)),
        spec!(
            "view.contextMenu",
            "Show context menu",
            "{}  the selection's context menu at its layer row (when the layers panel has the keyboard) or on the canvas; the menu takes the keyboard",
            false,
            |s, _| ui(s, "contextMenu")
        ),
        spec!("view.themeLight", "Light theme", "{}", false, |s, _| {
            s.prefs.theme = "light".into();
            ok()
        }),
        spec!("view.themeDark", "Dark theme", "{}", false, |s, _| {
            s.prefs.theme = "dark".into();
            ok()
        }),
        spec!("view.themeSystem", "Follow system theme", "{}", false, |s, _| {
            s.prefs.theme = "system".into();
            ok()
        }),
        spec!("pref.keepTool", "Keep tool selected after use", "{on?}", false, |s, p| pref(s, p, |x| &mut x.keep_tool)),
        spec!("pref.highlightHover", "Highlight layers on hover", "{on?}", false, |s, p| pref(s, p, |x| &mut x.highlight_on_hover)),
        spec!("pref.renameDuplicates", "Rename duplicated layers", "{on?}", false, |s, p| pref(s, p, |x| &mut x.rename_duplicates)),
        spec!("pref.showDimensions", "Show dimensions on objects", "{on?}", false, |s, p| pref(s, p, |x| &mut x.show_dimensions)),
        spec!("pref.numberKeysOpacity", "Use number keys for opacity", "{on?}", false, |s, p| pref(s, p, |x| &mut x.number_keys_opacity)),
        spec!("pref.reduceMotion", "Reduce motion", "{on?}", false, |s, p| pref(s, p, |x| &mut x.reduce_motion)),
        spec!(
            "pref.uiScale",
            "Interface size",
            "{scale?: a size from 0.75 to 3 relative to the system's display scale, 1 being the system's own size, so 1.25 is 125 %, not 125 (outside the range it stops at the nearest end; \"auto\" is 1; omit: just read it)} → {scale}: how large the whole interface is drawn; the canvas zoom is separate",
            false,
            |s, p| {
                match get(p, "scale") {
                    None => {}
                    Some(Value::String(v)) if v.trim().eq_ignore_ascii_case("auto") => s.prefs.ui_scale = 1.0,
                    Some(_) => {
                        let scale = req_num("pref.uiScale", p, "scale")?;
                        s.prefs.ui_scale = crate::clamp_ui_scale(scale).ok_or_else(|| bad("pref.uiScale", "`scale` must be a finite number"))?;
                    }
                }
                Ok(json!({"scale": s.prefs.ui_scale}))
            }
        ),
        spec!("pref.autoSave", "Save files automatically", "{on?}", false, |s, p| pref(s, p, |x| &mut x.auto_save)),
        spec!("pref.startOnHome", "Start on the Home screen", "{on?}", false, |s, p| pref(s, p, |x| &mut x.start_on_home)),
        spec!("pref.nudge", "Nudge amount", "{small?, big?}", false, |s, p| {
            s.prefs.nudge_small = num(p, "small").unwrap_or(s.prefs.nudge_small).clamp(0.01, 1000.0);
            s.prefs.nudge_big = num(p, "big").unwrap_or(s.prefs.nudge_big).clamp(0.01, 10000.0);
            Ok(json!({"small": s.prefs.nudge_small, "big": s.prefs.nudge_big}))
        }),
        spec!("pref.userName", "Your name in comments", "{name (blank: the default, \"You\")}", false, |s, p| {
            let name = req_string("pref.userName", p, "name")?;
            let name: String = name.trim().chars().filter(|c| !c.is_control()).take(64).collect();
            s.prefs.user_name = if name.is_empty() { super::comment::DEFAULT_AUTHOR.into() } else { name };
            Ok(json!({"name": s.prefs.user_name}))
        }),
        spec!(
            "pref.publishServer",
            "Server for online sharing",
            "{url?: ws://host:port/ or wss://host/path/ (no key; empty: online sharing off; omit: back to the built-in server)} → {url, default (the built-in server)}: where Share ▸ Online keeps files and where join codes are asked",
            false,
            |s, p| {
                const CMD: &str = "pref.publishServer";
                let url = match get(p, "url") {
                    None => crate::live::DEFAULT_SERVER.to_string(),
                    Some(_) => req_string(CMD, p, "url")?.trim().to_string(),
                };
                let web_socket = url.starts_with("ws://") || url.starts_with("wss://");
                if !url.is_empty()
                    && (!web_socket || url.len() > 500 || url.contains(|c: char| c.is_whitespace() || c.is_control() || c == '?' || c == '#'))
                {
                    return Err(bad(
                        CMD,
                        "the server's address is ws://host:port/ or wss://host/path/, without a key (it is saved with your preferences)",
                    ));
                }
                s.prefs.publish_server = url;
                Ok(json!({"url": s.prefs.publish_server, "default": crate::live::DEFAULT_SERVER}))
            }
        ),
        spec!(
            "pref.language",
            "Interface language",
            "{language: system|en|cs|ru|es|es-419|fr|de|pt-BR|ja|ka|ko|zh-Hans|…; else English}",
            false,
            |s, p| {
                let lang = req_string("pref.language", p, "language")?;
                let lang = lang.trim();
                // A BCP 47-style tag ("es", "pt-BR", "zh_Hans") or "system".
                let tag = lang.len() <= 35
                    && lang.split(['-', '_']).all(|part| !part.is_empty() && part.len() <= 8 && part.chars().all(|c| c.is_ascii_alphanumeric()));
                if !tag {
                    return Err(bad("pref.language", format!("`{lang}` is not a language code")));
                }
                s.prefs.language = lang.to_string();
                Ok(json!({"language": s.prefs.language}))
            }
        ),
        spec!("app.preferences", "Preferences", "{}", false, |s, _| ui(s, "dialog.preferences")),
        spec!("app.actions", "Actions", "{}", false, |s, _| ui(s, "palette.actions")),
        spec!("help.shortcuts", "Keyboard shortcuts", "{}", false, |s, _| ui(s, "panel.shortcuts")),
        spec!("help.about", "About FigCraft", "{}", false, |s, _| ui(s, "dialog.about")),
        spec!("help.page", "Help page", "{}", false, |s, _| ui(s, "open.help")),
        spec!(
            "help.gettingStarted",
            "Getting started",
            "{show? (default true; false closes them)}  the Getting started tips on the home screen: shows them again (the app opens the home screen), or closes them until asked for → {shown}",
            false,
            |s, p| {
                let show = boolean(p, "show").unwrap_or(true);
                s.prefs.getting_started_dismissed = !show;
                if show {
                    s.request_ui("home.gettingStarted", json!({}));
                }
                Ok(json!({"shown": show}))
            }
        ),
        spec!("libraries.open", "Libraries", "{}", false, |s, _| ui(s, "panel.libraries")),
        spec!(
            "tool.set",
            "Choose tool",
            "{tool: move|scale|hand|frame|section|slice|rectangle|line|arrow|ellipse|polygon|star|pen|pencil|text|comment|shapeBuilder | on boards: sticky|shape|connector|marker|highlighter|eraser|stamp}",
            false,
            |s, p| {
                let t = req_string("tool.set", p, "tool")?;
                let t = ToolId::from_id(&t).ok_or_else(|| bad("tool.set", format!("unknown tool `{t}`")))?;
                if t.on_boards() {
                    crate::board::require(s, &format!("tool.{}", t.id()))?;
                }
                tool(s, t)
            }
        ),
        spec!(
            "tool.place",
            "Place with the tool",
            "{tool? (a shape or frame tool; default the current one), x?, y? (its centre, page coords; default the middle of the view)} — what a click with the tool makes, selected (Enter with the tool chosen)",
            false,
            |s, p| {
                if let Some(t) = string(p, "tool") {
                    let t = ToolId::from_id(&t).ok_or_else(|| bad("tool.place", format!("unknown tool `{t}`")))?;
                    tool(s, t)?;
                }
                let at = num(p, "x").zip(num(p, "y")).map(|(x, y)| figcraft_geom::Point::new(x, y));
                s.place(at)?;
                Ok(json!({"ids": s.selection()}))
            }
        ),
        spec!(
            "tool.pencilBrush",
            "Pencil brush",
            "{preset? (taper|ink|ribbon|bristle|dots|stars|confetti|leaves | null: plain), spacing?, sizeJitter?, rotationJitter?, scatter?, seed?} (the brush new Pencil strokes get; omit `preset` to read it)",
            false,
            |s, p| {
                // `null` (unlike a missing preset) clears the brush.
                if let Some(v) = p.get("preset") {
                    let v = if v.is_null() { Value::Null } else { p.clone() };
                    s.tool.pencil_brush = super::props::brush_of(&v).ok_or_else(|| bad("tool.pencilBrush", super::props::BRUSH_HELP))?;
                }
                Ok(json!({"brush": s.tool.pencil_brush}))
            }
        ),
        spec!(
            "tool.scaleAnchor",
            "Scale anchor",
            "{anchor?: tl|t|tr|l|c|r|bl|b|br} (the point of the selection that stays put when the Scale tool scales; omit to read it)",
            false,
            |s, p| {
                if let Some(a) = string(p, "anchor") {
                    s.tool.scale_anchor = crate::tools::anchor_of(&a).ok_or_else(|| bad("tool.scaleAnchor", format!("unknown anchor `{a}`")))?;
                    if let Ok(ds) = s.ds_mut() {
                        ds.revision += 1;
                    }
                }
                Ok(json!({"anchor": crate::tools::anchor_name(s.tool.scale_anchor)}))
            }
        ),
        spec!("tool.move", "Move", "{}", false, |s, _| tool(s, ToolId::Move)),
        spec!("tool.scale", "Scale", "{}", false, |s, _| tool(s, ToolId::Scale)),
        spec!("tool.hand", "Hand", "{}", false, |s, _| tool(s, ToolId::Hand)),
        spec!("tool.frame", "Frame", "{}", false, |s, _| tool(s, ToolId::Frame)),
        spec!("tool.section", "Section", "{}", false, |s, _| tool(s, ToolId::Section)),
        spec!("tool.slice", "Slice", "{}", false, |s, _| tool(s, ToolId::Slice)),
        spec!("tool.rectangle", "Rectangle", "{}", false, |s, _| tool(s, ToolId::Rectangle)),
        spec!("tool.line", "Line", "{}", false, |s, _| tool(s, ToolId::Line)),
        spec!("tool.arrow", "Arrow", "{}", false, |s, _| tool(s, ToolId::Arrow)),
        spec!("tool.ellipse", "Ellipse", "{}", false, |s, _| tool(s, ToolId::Ellipse)),
        spec!("tool.polygon", "Polygon", "{}", false, |s, _| tool(s, ToolId::Polygon)),
        spec!("tool.star", "Star", "{}", false, |s, _| tool(s, ToolId::Star)),
        spec!("tool.pen", "Pen", "{}", false, |s, _| tool(s, ToolId::Pen)),
        spec!("tool.pencil", "Pencil", "{}", false, |s, _| tool(s, ToolId::Pencil)),
        spec!("tool.text", "Text", "{}", false, |s, _| tool(s, ToolId::Text)),
        spec!("tool.comment", "Comment", "{}", false, |s, _| tool(s, ToolId::Comment)),
        spec!("tool.shapeBuilder", "Shape builder", "{}", false, |s, _| tool(s, ToolId::ShapeBuilder)),
        // `apply: false` is for the UI colour picker: the eyedropper only samples (into
        // `ToolState::picked`) and the picker writes the colour to the paint it edits.
        spec!("edit.pickColor", "Pick color", "{apply?}", false, |s, p| {
            let r = tool(s, ToolId::Eyedropper)?;
            s.tool.pick_only = !boolean(p, "apply").unwrap_or(true);
            s.tool.picked = None;
            Ok(r)
        }),
    ]
}

#[allow(dead_code)]
fn unused(_: NodeId) {}

#[cfg(test)]
mod start_on_home_tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn start_on_home_is_a_preference_that_persists() {
        let mut s = Session::new();
        assert!(!s.prefs.start_on_home);
        assert_eq!(s.execute("pref.startOnHome", &json!({})).unwrap()["on"], true);
        assert!(s.prefs.start_on_home);
        // Saved with the other preferences and read back; older settings files without it read as off.
        let saved: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
        assert!(saved.start_on_home);
        assert_eq!(s.execute("pref.startOnHome", &json!({"on": false})).unwrap()["on"], false);
        let old: crate::Prefs = serde_json::from_value(json!({"theme": "dark"})).unwrap();
        assert!(!old.start_on_home);
    }
}
