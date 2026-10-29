//! FigCraft in the browser.
//!
//! Runs the same [`figcraft_ui_egui::FigApp`] as the desktop app through eframe's web runner
//! (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release` from this
//! directory; the site lands in `dist/web` (see `packaging/web/README.md`).
//!
//! Differences from the desktop app:
//! - no TCP control server (browsers can't listen on sockets);
//! - no native file dialogs yet: the session starts with one new document;
//! - links open in a new browser tab;
//! - of the preferences, the page keeps only your name across reloads (`kept.rs`);
//! - live sessions: it joins sessions a desktop app hosts (File ▸ Join live session…) but can't
//!   host one. A page served over `https://` can only reach sessions on the same computer
//!   (browsers block `ws://` from secure pages); serve it over `http://` on a trusted network to
//!   join from other machines.
//!
//! URL query flags: `?webgl` forces the WebGL2 backend instead of WebGPU; `?control` installs
//! `window.figcraft`, the page's control channel (`api.rs`), which is off without it.
//!
//! URL fragment: `#join=<invite link or join code>` joins that live session once the page has
//! loaded, so a bookmark opens a shared file. `join` takes the rest of the fragment: put it last.
//! The link carries the session's secret; a fragment is never sent to the web host, but the address
//! bar and the browser history keep it, so the URL is as private as the link itself.

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(any(target_arch = "wasm32", test))]
mod kept;
#[cfg(any(target_arch = "wasm32", test))]
mod query;
#[cfg(test)]
mod query_tests;
#[cfg(any(target_arch = "wasm32", test))]
mod saved_files;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("figcraft-web only runs in the browser: build it with `trunk build --release` in apps/figcraft-web");
}
