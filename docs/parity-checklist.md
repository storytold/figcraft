# Parity checklist

> **Generated:** 2026-11-28 by `cargo xtask parity`. Do not edit by hand.

FigCraft commands registered for the incumbent's main menu and toolbar (667 commands in the engine).

**215 / 215 menu items (100.0%)**

Every catalog item has a command.

## Headless

**Headless ([RFC-0052](https://github.com/storytold/craft-contrib/blob/main/rfcs/0052-headless-reach.md)): 210 yes, 2 no (window 0, os 2, engine-gap 0), 3 partial, 0 unknown.**

An item is `headless: yes` when its id is in the engine registry that `figcraft-cli` and the MCP server load. The UI-layer ids that light up a menu item above don't count. Other values are the hand marks in `HEADLESS` (`crates/engine/src/catalog.rs`), or `unknown`.

- `edit.copyAsText` Copy as text: headless: no, reach: os (puts the selected layers' text on the OS clipboard)
- `edit.copyAsCss` Copy as CSS: headless: partial, reach: os (`inspect.code` with `language: css` returns the same CSS; the clipboard is the OS's)
- `edit.copyAsSvg` Copy as SVG: headless: partial, reach: os (`file.exportSvg` writes the same SVG to a file; the clipboard is the OS's)
- `edit.copyAsPng` Copy as PNG: headless: partial, reach: os (`file.export` writes the PNG to a file; the clipboard is the OS's)
- `edit.copyLink` Copy link to selection: headless: no, reach: os (puts a `figcraft://` link on the OS clipboard)

Menu coverage measures breadth only; see docs/target-app-parity.md for depth and fidelity.
