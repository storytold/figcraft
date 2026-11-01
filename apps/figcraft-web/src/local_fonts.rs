//! Fonts installed on this computer, through the browser's Local Font Access API
//! (`window.queryLocalFonts`, Chromium): asked for when the person clicks (the browser asks them
//! for permission), then known by name in the font list. A font's bytes are read only when text
//! uses it: `figcraft_text` asks for them ([`load_wanted`], each frame) and takes them when they
//! come; auto-sizing text then fits the real font.

use std::cell::{Cell, RefCell};

use figcraft_text::LocalFont;
use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Most fonts taken from the browser's list (a computer with a font manager can have tens of
/// thousands; each is only a name here until it is used).
const MAX_FONTS: usize = 20_000;

thread_local! {
    /// The browser's `FontData` of each font offered, by its number.
    static FONTS: RefCell<Vec<JsValue>> = const { RefCell::new(Vec::new()) };
    /// Asked for already (the browser asks the person once; their answer stands for the page).
    static ASKED: Cell<bool> = const { Cell::new(false) };
    /// A font's bytes came since the last frame: auto-sizing text fits them.
    static LOADED: Cell<bool> = const { Cell::new(false) };
}

/// The browser can list the fonts installed on this computer, and the page hasn't asked yet.
pub fn can_ask() -> bool {
    !ASKED.with(Cell::get) && web_sys::window().is_some_and(|w| Reflect::has(&w, &"queryLocalFonts".into()).unwrap_or(false))
}

/// Ask the browser for the installed fonts (it asks the person); `ctx` repaints when they're in.
pub fn ask(ctx: egui::Context) {
    ASKED.with(|a| a.set(true));
    wasm_bindgen_futures::spawn_local(async move {
        match query().await {
            Ok(n) => log::info!("fonts on this computer: {n} new families"),
            Err(e) => log::warn!("the browser didn't list the fonts on this computer: {e:?}"),
        }
        ctx.request_repaint();
    });
}

/// A `FontData`'s text field.
fn text(font: &JsValue, key: &str) -> String {
    Reflect::get(font, &key.into()).ok().and_then(|v| v.as_string()).unwrap_or_default()
}

/// Call `obj.name()`, a method returning a promise, and wait for it.
async fn call(obj: &JsValue, name: &str) -> Result<JsValue, JsValue> {
    let f: Function = Reflect::get(obj, &name.into())?.dyn_into()?;
    JsFuture::from(f.call0(obj)?.dyn_into::<Promise>()?).await
}

/// The browser's font list, offered to the font database by name: the number of new families.
async fn query() -> Result<usize, JsValue> {
    let window: JsValue = web_sys::window().ok_or("no window")?.into();
    let list: Array = call(&window, "queryLocalFonts").await?.dyn_into()?;
    let fonts = FONTS.with(|all| {
        let mut all = all.borrow_mut();
        list.iter()
            .take(MAX_FONTS.saturating_sub(all.len()))
            .map(|font| {
                let f = LocalFont {
                    id: all.len(),
                    family: text(&font, "family"),
                    style: text(&font, "style"),
                    postscript: text(&font, "postscriptName"),
                };
                all.push(font);
                f
            })
            .collect()
    });
    Ok(figcraft_text::add_local_fonts(fonts).len())
}

/// Read the fonts text asked for since the last frame; `ctx` repaints as each comes.
pub fn load_wanted(ctx: &egui::Context) {
    for id in figcraft_text::take_wanted_local_fonts() {
        let Some(font) = FONTS.with(|f| f.borrow().get(id).cloned()) else { continue };
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            // A font that can't be read is dropped (its text keeps the fallback).
            let bytes = read(&font).await.unwrap_or_else(|e| {
                log::warn!("couldn't read a font from this computer: {e:?}");
                Vec::new()
            });
            if figcraft_text::load_local_font(id, bytes).is_some() {
                LOADED.with(|l| l.set(true));
            }
            ctx.request_repaint();
        });
    }
}

/// A `FontData`'s file, at most [`figcraft_text::fontdb::MAX_FONT_FILE`] bytes.
async fn read(font: &JsValue) -> Result<Vec<u8>, JsValue> {
    let blob: web_sys::Blob = call(font, "blob").await?.dyn_into()?;
    if blob.size() > figcraft_text::fontdb::MAX_FONT_FILE as f64 {
        return Err("the font file is too large".into());
    }
    let buf = JsFuture::from(blob.array_buffer()).await?;
    Ok(Uint8Array::new(&buf).to_vec())
}

/// A font's bytes came since the last call.
pub fn take_loaded() -> bool {
    LOADED.with(|l| l.replace(false))
}
