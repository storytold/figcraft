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
//! - an internal error (a panic) can't be caught, so it stops the page: the files with unsaved
//!   changes are kept in the page's storage and open again on the next reload (`recovery.rs`);
//! - fonts: the built-in ones, and on request (the font menu's "Use fonts installed on this
//!   computer…", Chromium's Local Font Access) the computer's, each read when text uses it
//!   (`local_fonts.rs`);
//! - live sessions: it joins sessions a desktop app hosts (File ▸ Join live session…) and files on
//!   a `figcraft-server`, and publishes files to a server (Share ▸ Online, Share ▸ Publish to
//!   server…), but can't host a session. A page served over `https://` reaches only `wss://`
//!   servers and the same computer (browsers block `ws://` from secure pages); serve it over
//!   `http://` on a trusted network to reach plain `ws://` hosts on other machines.
//! - host mode: `?host=parent` (a same-origin page embedding FigCraft in an `<iframe>`) or
//!   `data-host="page"` on the canvas (the page's own document) hands the page the app's files
//!   instead of downloading them and reports unsaved changes (`host`);
//! - live sessions: it joins sessions a desktop app hosts (File ▸ Join live session…) but can't
//!   host one. A page served over `https://` can only reach sessions on the same computer
//!
//! URL query flags: `?webgl` forces the WebGL2 backend instead of WebGPU; `?control` installs
//! `window.figcraft`, the page's control channel (`api.rs`), which is off without it; `?host=parent`
//! (inside an `<iframe>`) talks to a same-origin parent page instead: it sends the document to open
//! and stores what FigCraft saves. `&author=` names the user for comments and board notes in either
//! host mode.
//!
//! URL fragment: `#join=<invite link or join code>` joins that live session once the page has
//! loaded, so a bookmark opens a shared file. `join` takes the rest of the fragment: put it last.
//! The link carries the session's secret; a fragment is never sent to the web host, but the address
//! bar and the browser history keep it, so the URL is as private as the link itself.

#[cfg(target_arch = "wasm32")]
mod api;
#[cfg(target_arch = "wasm32")]
mod host;
#[cfg(any(target_arch = "wasm32", test))]
mod hostmode;
#[cfg(any(target_arch = "wasm32", test))]
mod kept;
#[cfg(target_arch = "wasm32")]
mod local_fonts;
#[cfg(target_arch = "wasm32")]
mod pasted;
#[cfg(any(target_arch = "wasm32", test))]
mod query;
#[cfg(test)]
mod query_tests;
#[cfg(any(target_arch = "wasm32", test))]
mod recovery;
#[cfg(test)]
mod recovery_tests;
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
