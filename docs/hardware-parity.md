# Hardware and platform parity

> **Last reviewed:** 2026-10-10 · **Last updated:** 2026-10-18 · **Change:** minor (macOS: quitting ends the process once everything is kept, avoiding the Touch Bar teardown crash (#781); Linux and BSD clipboard: images under Wayland, reads time out; minimised windows under Wayland; window controls on Wayland without server decorations; canvas textures on device pixels; copy and paste in the browser; Windows QA pass; Display P3: gamut warning and soft-proof (true P3 blocked upstream); several windows; pen pressure for the Pencil and brushes on Windows and from browser styluses) · **Target:** Figma Design (desktop app 126.10.7)

How FigCraft uses hardware and which platforms it runs on, against Figma. Figma's desktop app is
Electron with a GPU helper process; its canvas renders on the GPU (WebGL, with WebGPU rolling out
since 2025). FigCraft renders the canvas on the CPU and presents it through wgpu.

**Summary (estimated):** hardware ~60%, performance ~60%, platforms ~75%.

## Hardware

| Feature | Figma | FigCraft (macOS / Windows / Linux / web) | Hours |
|---|---|---|---|
| Canvas rendering | GPU | CPU (tiny-skia), banded across cores, retained (re-renders only what an edit or pan touches), levels of detail below 50% zoom; shown through wgpu (WebGPU or WebGL2 in the browser) | 30–60 for a GPU path |
| Effects (blurs, glass, noise) | GPU | CPU | in the above |
| HiDPI / Retina | yes | yes (all); a canvas image rendered at the screen's density is placed on whole device pixels, so fractional pans, zooms and display scales don't blur it (#359; geometry tests, the snapping not judged by eye) | — |
| Wide-gamut display (Display P3) | yes, on capable displays | colours stored and exported in P3; the screen shows sRGB: eframe/egui-wgpu 0.36 configure an 8-bit sRGB surface with no colour space option, although wgpu 30 can present Display P3 or extended sRGB on Metal, DX12, Vulkan and WebGPU (see gaps #18). The colour picker warns about colours outside sRGB and View ▸ Soft-proof sRGB stripes them on the canvas | 2–5 |
| Trackpad pinch zoom and two-finger pan | yes | yes (egui zoom delta, smooth scroll) | — |
| Pen tablet pressure (pencil, Figma Draw brushes) | yes (forum reports of regressions in 2025) | Windows pens (winit pointer input) and browser styluses that send touch events with a force (an Apple Pencil in Safari); the Pencil records it as the stroke's width profile. A mouse, macOS and Linux tablets and pens sending only pointer events in a browser report none: constant width | 1–3 |
| Several windows / multi-monitor | yes (a window per file; presenter in its own window) | desktop: File ▸ New window, a tab's Open in new window or a tab dragged off the strip; each window has its own tabs, panels and canvas, draws only when it needs to and asks about unsaved changes when it closes. Only the main window has a screen-reader tree (eframe 0.36), so files stay tabs while a screen reader runs; positions aren't restored at the next start; the presenter stays inside its window; one window in the browser | 1–2 |
| Screen readers | partial | AccessKit tree on desktop (names, roles, states), never tested with a real screen reader; none in the browser | 4–8 + human QA |
| Keyboard layouts (non-US) | yes | keymap uses logical keys; not checked on AZERTY/QWERTZ/JIS | 1–2 |

## Performance (measured by `cargo xtask bench`, synthetic files, owner's Mac)

| Case | FigCraft | Figma |
|---|---|---|
| 50k-layer page, zoomed out, full render | ~8 ms (levels of detail) | not timed side by side |
| 10k layers: full 1440×900 render | 7–15 ms | not timed |
| 10k layers: move one layer, re-render | ~0.2 ms | not timed |
| Design system of ~300 components × 2,000 instances: recolour a main used ~900 times | 4–18 ms | not timed |
| `.figcraft` at 50k layers: save / load | ~0.09 s / ~0.16 s | n/a |
| Idle window | 0 frames, ~0% CPU | — |

Nothing has been timed against Figma on the same document (needs `.fig` import). 

## Platforms

| Platform | Figma | FigCraft | Notes |
|---|---|---|---|
| macOS | desktop app (Intel and Apple silicon) | universal `.dmg`, signed and notarized (v0.2.0) | where nearly all QA happens. Quitting ends the process once the preferences, recovery copies and live sessions are taken care of: the window's teardown crashed on Macs with a Touch Bar (#781); Quit from the menu bar, which asks nothing, keeps unsaved files' recovery copies for the next start |
| Windows x64 / arm64 | desktop app | `.msi` and portable zip for x64, x86 and arm64 | built in CI; Windows: QA pass done 2026-10-11, findings in PR |
| Linux | none (browser only) | AppImage, `.deb`, `.rpm`, Flatpak, tarball, x86_64 and aarch64; riscv64 tarball | CI tests run here. Clipboard (also the BSDs): reads run on a helper thread that gives up after 2 s, and images copied in other apps paste under Wayland through the data-control protocol (#693). Under Wayland the app can't tell its window is minimised, so control requests that need a drawn frame may still wait there (#720). None of this run on a Linux desktop here. On a Wayland compositor without server-side decorations (GNOME) the window gets winit's Adwaita controls (#592: checked by its author in a GNOME Wayland session; KDE not run) |
| FreeBSD | none | tarball | |
| Web | primary (figma.com) | `figcraft-web` (WASM, WebGPU/WebGL2), web zip | no accessibility tree; file access through the browser; copy and paste through the system clipboard (copies written from the copy event, pastes read from the paste event; no pasting images from other apps) |
| iOS / Android | apps: view, comment, mirror prototypes | none (the web build is not touch-first) | 20–40 h for a viewer |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-18 | minor | macOS quit crash (#781): on Macs with a Touch Bar the app could crash as it quit (an exception in AppKit's Touch Bar support while the window's content view, which the accessibility adapter subclasses, was torn down). Quitting now keeps everything (preferences, this run's recovery copies, live sessions ended) and then ends the process before that teardown. Quit from the menu bar ends the app without the unsaved-changes question, so unsaved files' recovery copies now stay and reopen at the next start instead of being removed. Numbers unchanged |
| 2026-10-17 | minor | Linux row: clipboard reads on Linux and the BSDs time out after 2 s on a helper thread, images copied in other apps paste under Wayland (arboard's data-control feature, #693), a minimised window can't be detected under Wayland, so frame-only control requests may still wait there (#720), and Wayland compositors without server-side decorations give the window Adwaita controls (#592, checked by its author on GNOME, not by the maintainers; KDE not run); compiled in CI, not run on a Linux desktop. HiDPI row: native-density canvas images placed on whole device pixels (#359, geometry tests; not judged by eye). Numbers unchanged |
| 2026-10-14 | minor | Copy and paste in the browser: the web build never wrote to the system clipboard, so Cmd+V after Cmd+C pasted whatever text the clipboard held before (as a text layer) instead of the copied layers. Copies (layers, Copy as SVG/CSS/text/PNG, links) now go out through egui's output, which eframe writes during the copy event (`web_clipboard_tests`). Numbers unchanged. |
| 2026-10-11 | minor | Windows QA pass (gap 8): everyday workflows on Windows 11 through the desktop app and MCP; fixed: unsaved changes lost when the app was killed or crashed (recovery copies every few seconds, reopened at the next start), Ctrl+arrows/Backspace/Delete acting on the whole line in text (now by word; Ctrl+Home/End to the text's ends), scrubbed fields snapping back on release, typing on a field Tab reached, Escape not closing dialogs, skewed or vanishing linear gradients, hints cut off at the panel edge, the window-close question in English in the Spanish interface. Percentages and hours unchanged |
| 2026-10-11 | minor | Display P3 honest minimum (gap 18): the colour picker of a Display P3 file warns when a colour is outside sRGB (Outside sRGB, with what the canvas and exports do), and View ▸ Soft-proof sRGB (`view.proofSrgb`) stripes the canvas pixels sRGB can't show. Why the canvas still shows sRGB: wgpu 30 can present Display P3 or extended-range sRGB surfaces (Metal, DX12, Vulkan, WebGPU canvases), but eframe/egui-wgpu 0.36 configure an 8-bit sRGB surface with no option for the format or colour space, and egui's renderer writes sRGB-encoded UI colours; showing P3 needs those two upstream (or a patched egui-wgpu). Hardware 13–27 h → 12–26 h; gap 18 6–12 h → 5–11 h; percentages, feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~363–697 h → ~362–696 h |
| 2026-10-11 | minor | Several windows (gap 18): File ▸ New window, a tab's Open in new window or a tab dragged off the tab strip opens files in another window with its own tabs, panels and canvas, sharing the files, preferences, libraries and clipboard; each window draws only when it needs to (idle: ~0% CPU), closing one closes its files after asking about unsaved changes (the main window too, which takes over another window instead of quitting; the last window quits), and the control channel and MCP open, list, focus and capture windows (`window.new`, `window.list`, `window.focus`, `window` on any request); the screen-reader tree covers the main window only (eframe 0.36), so with a screen reader running files stay tabs; one window in the browser. Hardware 55% → 60%, 13–27 h → 10–21 h; gap 18 6–12 h → 4–8 h (P3 on screen left); feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~363–697 h → ~360–691 h |
| 2026-10-11 | minor | Stretch and scatter brushes and pen pressure landed (gap 9): eight brushes drawn in code (four stretched along the path, four scattered with spacing, size, rotation and scatter jitter from a seed) in the stroke settings, drawn everywhere strokes are and flattened by outline stroke; the Pencil records pen pressure as the stroke's width profile (Windows pens, browser styluses; macOS and Linux tablets report none) and can draw with a brush; Figma Draw extras breadth 45% → 65%, ready 30% → 45%, 10–18 h → 5–9 h; Strokes 2–4 h → 1–3 h; Hardware 50% → 55%, 15–30 h → 13–27 h; feature breadth (~79%) and ready for real work (~47%) unchanged at this rounding; full parity ~380–725 h → ~375–715 h |
| 2026-10-10 | major | Created from the 2026-10-10 re-measure |
