<p align="center">
  <a href="https://getartcraft.com/">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/storytold/vectorcraft/main/docs/brand/artcraft-logo-white.png">
      <img alt="ArtCraft" src="https://raw.githubusercontent.com/storytold/vectorcraft/main/docs/brand/artcraft-logo.png" width="200">
    </picture>
  </a>
</p>

<p align="center">
  <img src="assets/app-icon/hicolor/256x256/apps/ai.storyteller.figcraft.png" width="128" height="128" alt="FigCraft icon: an engraved chameleon on a mint-green tile">
</p>

<h1 align="center">FigCraft</h1>

<p align="center">
  <b>Interface design and prototyping: open source, collaborative and built in pure Rust.</b><br>
  Frames, auto layout, components and variants, vector networks, styles, variables and prototypes,<br>
  in a native app that opens instantly and works offline. Open source, and yours.
</p>

<p align="center">
  <img alt="100% Rust" src="https://img.shields.io/badge/100%25-Rust-b7410e?style=flat-square&logo=rust">
  <img alt="macOS · Windows · Linux · FreeBSD · Web" src="https://img.shields.io/badge/macOS%20%C2%B7%20Windows%20%C2%B7%20Linux%20%C2%B7%20FreeBSD%20%C2%B7%20Web-native-0EA5C6?style=flat-square">
  <img alt="License: MIT OR Apache-2.0" src="https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-3a3a3a?style=flat-square">
  <img alt="Status: early alpha" src="https://img.shields.io/badge/status-early%20alpha-d69e2e?style=flat-square">
  <br>
  <a href="https://sonarcloud.io/project/overview?id=storytold_figcraft"><img alt="Scanned by SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>
  <a href="https://coderabbit.ai"><img alt="Reviewed by CodeRabbit" src="https://img.shields.io/coderabbit/prs/github/storytold/figcraft?style=flat-square&label=Reviewed%20by%20CodeRabbit&logo=coderabbit"></a>
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<p align="center">
  <a href="https://getartcraft.com/apps/figcraft"><b>FigCraft on getartcraft.com</b></a> ·
  <a href="https://getartcraft.com/">ArtCraft</a> ·
  <a href="https://getartcraft.com/apps">All Crafting Apps</a>
</p>

<br>

<p align="center">
  <img src="docs/images/shot-1-hero.png" alt="FigCraft in its dark theme editing the Ridgeline hiking app: three phone screens (Home in light mode, a trail detail page and Home in dark mode) on the canvas, the Featured trail card instance selected, the layers panel showing the screen's auto layout tree, and the Design panel showing the instance's Fill and Hug auto layout sizing, its Theme variable mode and a fill bound to the surface colour variable" width="100%">
  <br><sub><b>Ridgeline</b>: auto layout screens built from components, with Light and Dark modes from one set of colour variables · <code>examples/ridgeline-app.figcraft</code></sub>
</p>

<br>

> [!NOTE]
> **ArtCraft is a community of artists from all walks of life.** Designers, illustrators, developers,
> musicians, game makers &mdash; if you make things, you're one of us. **[Come say hi on Discord](https://discord.gg/artcraft).**

<p align="center">
  <a href="#a-look-around">A look around</a> ·
  <a href="#made-in-figcraft">Made in FigCraft</a> ·
  <a href="#features">Features</a> ·
  <a href="#built-for-agents">Agents</a> ·
  <a href="#under-the-hood">Under the hood</a> ·
  <a href="#get-started">Get started</a> ·
  <a href="#the-crafting-apps">Crafting Apps</a> ·
  <a href="https://discord.gg/artcraft">Discord</a>
</p>

<br>

<table>
  <tr>
    <td width="25%" valign="top">
      <h3>🎛️ Familiar by design</h3>
      The toolbar, layers panel, Design panel, menus and shortcuts sit right where your hands expect them.
    </td>
    <td width="25%" valign="top">
      <h3>⚡ Native and fast</h3>
      A Rust engine with a persistent document model: edits touch only what changed, so creating a node in a 20,000-node file takes a fraction of a millisecond. No Electron, no web view.
    </td>
    <td width="25%" valign="top">
      <h3>🔌 Offline and private</h3>
      Your files are plain <code>.figcraft</code> archives on your own disk. No account, no telemetry, and nothing leaves your computer unless you share a file (<a href="#privacy-and-online-sharing">what sharing sends where</a>). SVG, PNG, JPG, WebP and PDF in and out; <code>.fig</code> files open (best effort), and so do designs exported as the public REST API file JSON, with their prototypes and variables.
    </td>
    <td width="25%" valign="top">
      <h3>🤖 Agent-ready</h3>
      Every menu item, tool and drag is a command, so the same engine runs from the UI, the CLI, a JSON control channel or an MCP server.
    </td>
  </tr>
</table>

<br>

## A look around

<table>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/shot-2-components.png" alt="The Ridgeline UI design system open with the Assets panel on the left showing live thumbnails of the Button, Difficulty, Search field, Toggle and icon components; a Button component set of nine variants laid out in a three-by-three grid auto layout is selected, and the Design panel lists its Style (Primary, Tonal, Outline) and State (Default, Hover, Disabled) variant properties" width="100%">
  <p align="center"><sub><b>Components and variants</b>: component sets, variant properties and the Assets panel · <code>examples/ridgeline-design-system.figcraft</code></sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/shot-3-prototype.png" alt="Four onboarding phone screens in the Prototype tab, joined by blue connection curves from the Get started, Continue, Allow location and Not now buttons to the next screens; the Get started button is selected and the Prototype panel shows its On click, Navigate to 2 Interests interaction" width="100%">
  <p align="center"><sub><b>Prototyping</b>: connections, transitions, back actions and a flow · <code>examples/ridgeline-onboarding.figcraft</code></sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/shot-4-devmode.png" alt="Dev Mode on the Ridgeline home screen: the frame carries a Ready for dev badge, a Development annotation callout points at the featured trail card, a trail row instance is selected and the Inspect panel shows generated React code with a TrailRow props interface and the instance's title and details values" width="100%">
  <p align="center"><sub><b>Dev Mode</b>: ready-for-dev status, annotations and React code where component properties become props · <code>examples/ridgeline-app.figcraft</code></sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/shot-5-vector.png" alt="Vector edit mode on a wave path with a gradient, tapered variable-width stroke: its anchor points, Bezier handles and the transform box of the selected points are visible, with the vector edit toolbar at the bottom; above it are a boolean-built mountain mark and an angular-gradient knot of excluded rings" width="100%">
  <p align="center"><sub><b>Vector networks</b>: points and handles, variable-width strokes, booleans that stay editable · <code>examples/glass-and-vectors.figcraft</code></sub></p>
</td>
</tr>
<tr>
<td width="50%" valign="top">
  <img src="docs/images/shot-6-dashboard-light.png" alt="The light UI theme with a web analytics dashboard for a fictional trail operator: a dark sidebar, KPI cards with sparklines, a grouped bar chart, a donut chart, a table of top trails and an area chart, with the frame's 12-column layout grid showing as tinted columns and its fill linked to a paint style" width="100%">
  <p align="center"><sub><b>Layout grids and styles, light theme</b>: a 12-column grid, paint, text, effect and grid styles, charts drawn from shapes · <code>examples/ridgeline-studio-dashboard.figcraft</code></sub></p>
</td>
<td width="50%" valign="top">
  <img src="docs/images/shot-7-collab.png" alt="A live session seen from one participant's window: avatars of three people in the top right, a Priya Nair is spotlighting banner with a Follow button, Priya's green cursor and selection outline on the dark home screen's trail card, and Jonas Ortiz's purple cursor and selection on the Start hike button with his cursor chat bubble asking whether the button should stay pinned while scrolling" width="100%">
  <p align="center"><sub><b>Live sessions</b>: three apps on one <code>figcraft-server</code> file, with cursors, selections, cursor chat and spotlight</sub></p>
</td>
</tr>
</table>

## Made in FigCraft

Every design below was built entirely through FigCraft's command API, the same one the MCP server
exposes to agents, and exported at 2x by FigCraft's own renderer. The source files are in
[`examples/`](examples).

<table>
<tr>
<td width="33%" valign="top">
  <img src="docs/images/art-app-home.png" alt="Ridgeline home screen in light mode: a greeting, a search field, filter chips, a featured trail card with an illustrated mountain landscape, a list of nearby trails and a translucent tab bar" width="100%">
  <p align="center"><sub><b>Home, light</b></sub></p>
</td>
<td width="33%" valign="top">
  <img src="docs/images/art-app-trail.png" alt="Ridgeline trail detail screen: a dusk mountain illustration with glass buttons, a sheet with difficulty tags, the trail name, distance, elevation and duration stats, an elevation profile chart and a Start hike button" width="100%">
  <p align="center"><sub><b>Trail detail</b></sub></p>
</td>
<td width="33%" valign="top">
  <img src="docs/images/art-app-home-dark.png" alt="The same Ridgeline home screen in dark mode, its colours switched by the Theme variable collection's Dark mode" width="100%">
  <p align="center"><sub><b>Home, dark</b>: the same frame in the Dark mode</sub></p>
</td>
</tr>
<tr>
<td colspan="3" valign="top">
  <img src="docs/images/art-dashboard.png" alt="Ridgeline Studio dashboard: a dark navy sidebar with navigation, KPI cards for active hikers, check-ins, average rating and open incidents with sparklines, a hikes-per-day grouped bar chart with a tooltip, a difficulty donut chart, a top trails table with condition badges and a visitors area chart" width="100%">
  <p align="center"><sub><b>Ridgeline Studio</b>: a desktop dashboard on a 12-column grid, every chart made of shapes and vectors</sub></p>
</td>
</tr>
<tr>
<td colspan="2" valign="top">
  <img src="docs/images/art-glass-weather.png" alt="A glass weather widget over blurred teal, violet, rose and orange colour fields: a frosted card for Lake Louise showing 14 degrees and partly cloudy with a gradient sun behind a boolean-union cloud, an hourly forecast row, and smaller glass cards for air quality, wind and humidity" width="100%">
  <p align="center"><sub><b>Glass weather</b>: glass and background blur, layer-blurred colour fields, a boolean cloud</sub></p>
</td>
<td width="33%" valign="top">
  <img src="docs/images/art-vector-marks.png" alt="Three tiles on a cream background: a gradient mountain mark made by unioning two peaks and subtracting a sun, a knot of three excluded rings with an angular gradient, and a tapered gradient wave with a leaf and a glowing four-point star" width="100%">
  <p align="center"><sub><b>Vector marks</b>: union, subtract and exclude, an angular gradient, a variable-width stroke</sub></p>
</td>
</tr>
<tr>
<td colspan="3" valign="top">
  <img src="docs/images/art-ds-colour.png" alt="Ridgeline UI colour ramps: six swatches each of Pine greens, Sunset oranges and Slate greys, each labelled with its name and hex value" width="100%">
  <p align="center"><sub><b>Ridgeline UI</b>: colour ramps where every swatch is a paint style, exported through a slice</sub></p>
</td>
</tr>
</table>

## Features

<table>
  <tr>
    <td width="50%" valign="top">
      <h3>Layouts that build themselves</h3>
      Auto layout in both directions, with wrap and grid (fixed, hug and `fr` tracks, cells that span), padding, gaps, alignment including text baselines, Hug / Fill / Fixed sizing, min and max sizes, absolute positioning, drag handles for padding and spacing right on the canvas, and arrow keys that move children through the flow, grid cells and nested frames. Constraints and layout grids for everything else.
    </td>
    <td width="50%" valign="top">
      <h3>Components that scale</h3>
      Main components, instances with overrides that survive edits, component sets with variants, properties, swap instance, detach, reset overrides and go to main component, and an Assets panel with live thumbnails, search and drag-to-canvas into frames and auto layout. Local styles and variables (colour, number, string, boolean) with modes, aliases and scopes, edited in a Variables table and bound to fills, strokes, effects, layout grids and instance properties. Fills, strokes, effects and layout grids reorder by dragging their rows.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>Vectors without compromise</h3>
      Real vector networks: the Pen and Pencil tools, vector edit mode with its own tools (pen that extends and branches paths, bend, paint bucket fills for any closed area, lasso, cut), point panel with handle mirroring and per-point corner radius, join, split and heal-on-delete, the shape builder (click or drag across the regions of overlapping shapes, strokes and open paths to merge them, Alt to remove them), boolean groups (union, subtract, intersect, exclude) whose operands stay editable on the canvas with the result following every drag, an operation per operand, flatten, outline stroke, arrowheads, dashes, per-side strokes and variable width strokes (width profile presets, and a Variable width tool that drags width points along the path, one side at a time with Alt). Rectangles with smooth (squircle) corners, polygons, stars, and arcs and rings shaped with on-canvas sweep, start and ratio handles or the Design panel's Arc fields.
    </td>
    <td width="50%" valign="top">
      <h3>Paints and effects</h3>
      Solid, linear, radial, angular and diamond gradients with on-canvas handles, image fills (fill, fit, crop, tile) with adjustments and presets, multiple fills and strokes, every blend mode, drop and inner shadows (spread offsets the outline exactly), layer and background blur (uniform or progressive), noise (mono, duo, multi), texture and glass (refraction, dispersion, frost, light), masks. Files are sRGB or Display P3, with exports tagged with the file's colour profile.
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <h3>Type that sets beautifully</h3>
      Text edited in place on the canvas (with inline IME composition), mixed styles within a paragraph, auto width and auto height, line height, letter and paragraph spacing, decoration, case, lists, OpenType features, variable font axes and text styles, right-to-left and mixed-direction text (Arabic, Hebrew) with the Unicode bidi algorithm, shaped with HarfRust and the open fonts from craft-fonts plus your system fonts.
    </td>
    <td width="50%" valign="top">
      <h3>Prototype and hand off</h3>
      Connections, triggers, actions and Smart Animate-style transitions, variables with conditionals (if / else if / else built in the interaction editor, expressions like <code>count + 1</code>), flows, a presentation view with device frames, hotspot hints and an Inspect panel, and a live preview pane. Alt to measure distances and padding on the canvas, Inspect mode with measurements, annotations in colour-coded categories, a ready-for-dev list with a focus view and Compare changes (before / after with property-level diffs since a frame was marked ready), and code (CSS, HTML, React, Leptos, SwiftUI, Compose) where components take their properties as parameters, comment threads pinned to frames (replies, @mentions, reactions, read state) and named version history. Export any layer at any scale to PNG, JPG, WebP, SVG or PDF.
    </td>
  </tr>
</table>

<br>

## Built for agents

Every menu item, tool and gesture runs a command from one registry of 400+ commands. The UI, the CLI, the JSON control channel and the MCP server all call the same commands, so anything you can click, a script or an AI agent can do too.

```sh
# Headless: render and export
figcraft-cli render landing.figcraft -o landing.png --scale 2
figcraft-cli export landing.figcraft -o out/ --format svg

# Run a script of commands without a window
figcraft-cli run build-card.json --out card.figcraft

# Every command, with its parameters
figcraft-cli commands --filter autoLayout

# Let an agent drive it over MCP (headless, or attached to the running app)
figcraft-cli mcp
figcraft-cli mcp --attach 47800
```

The desktop app also offers a loopback-only control channel (`figcraft --control <port>`) for inspecting UI state, driving tools with pointer and key events, and taking screenshots. See [`docs/control-protocol.md`](docs/control-protocol.md), [`docs/mcp.md`](docs/mcp.md) and [`docs/cli.md`](docs/cli.md).

## Under the hood

- **Engine first:** a pure-data document model and a command engine, with a thin, swappable egui UI on top. Layering is checked at build time across the workspace crates (`cargo xtask layers`).
- **Persistent data:** the document lives in an immutable ordered map, so undo snapshots are cheap and layout, component sync and text measurement only revisit what changed.
- **Its own renderer:** tiny-skia on every core for the canvas and exports, with layer blending, effects and masks; booleans from linesweeper, geometry from kurbo, text from skrifa and HarfRust.
- **Runs in the browser:** the whole engine and UI compile to WebAssembly.
- **Accessible:** screen readers get names, roles, values and states for the tools, layer rows, fields, dropdowns, tabs and menus (AccessKit on desktop), F6 moves the keyboard between the toolbar, layers, canvas and properties (Tab within them, Escape back to the canvas), the system's reduce-motion setting is honoured, and panel text meets WCAG AA contrast in both themes.
- **Languages:** English, Russian and Turkish (reviewed by native speakers), and Spanish, French, German, Brazilian Portuguese, Japanese, Korean, Simplified Chinese, Czech, Indonesian and Georgian drafts, picked in Preferences or taken from the system or browser and switched live; adding a language is one catalog file ([`docs/translating.md`](docs/translating.md)).
- **Plugins:** sandboxed WebAssembly plugins (a metered, pure-Rust interpreter with a memory cap and no file or network access, in the browser too) that act through the same commands, each run one undo step, within the permissions their manifest asks for. A Rust SDK and three example plugins are in [`plugins/`](plugins); see [`docs/plugins.md`](docs/plugins.md).
- **Live sessions (preview):** share the open file from the app (Share ▸ Start sharing: online through a server, or on your local network), give the others the short join code it shows (or the invite link), and edit it together: everyone's changes sync live, with cursors, selections, avatars and follow, and undo takes back only your own changes. Others join from the desktop app (invite links open it) or from the browser build. See [`docs/collab-protocol.md`](docs/collab-protocol.md).
- **A server of your own (preview):** `figcraft-server` keeps a team's files, publishes them from the app (File ▸ Publish to server…, or Share ▸ Online with your server in Preferences) and hands out share links and join codes that let people edit or only view; files survive restarts and apps reconnect where they were. No accounts yet: links are the access. Releases ship it for Linux, and the desktop app joins over `wss://`; [hosting it on a rented Linux server](docs/server.md#deploy-on-a-linux-server) takes a systemd unit and Caddy or nginx (or Docker). See [`docs/server.md`](docs/server.md).
- **Never crashes:** no `unwrap`, `expect` or `panic` in shipped code, enforced by clippy; commands run behind a guard so a bug fails one action, not your session.
- **Clean-room:** implemented from public documentation and observed behaviour only. No proprietary code, file formats, icons or images.

## Get started

```sh
git clone https://github.com/storytold/figcraft
cd figcraft
cargo run --release -p figcraft            # the desktop app
cargo run --release -p figcraft-server     # the collaboration server (docs/server.md)
cargo test --workspace                     # the test suite
cargo xtask ci                             # fmt, clippy, tests, layering and asset checks
```

Fonts for the UI and the Text tool come from [craft-fonts](https://github.com/storytold/craft-fonts), cloned next to this repository (`../craft-fonts`); without it FigCraft falls back to your system fonts.

```sh
git clone https://github.com/storytold/craft-fonts ../craft-fonts
```

If you clone it after a first build, run `cargo clean -p figcraft-text` once so the next build embeds the fonts.

**Start from a template.** The home screen's Templates tab has eight starter files (a mobile app, a landing page, a dashboard, a design system starter, a wireframe kit, social post sizes, presentation slides and an icon set starter), also under File ▸ New from template…. They are built in code from original, synthetic content, so scripts and agents can open them too: `file.newFromTemplate {template}` ([`docs/mcp.md`](docs/mcp.md#starter-templates)).

For the web build, install [trunk](https://trunkrs.dev/) and run `cargo xtask wasm`.

New contributors and AI agents: start with [`AGENTS.md`](AGENTS.md), then [`docs/`](docs/).

Installers for macOS (universal, signed and notarized), Windows (x64, x86 and arm64), Linux (AppImage, `.deb`, `.rpm`, Flatpak and tarball), FreeBSD and the web are attached to each [GitHub release](https://github.com/storytold/figcraft/releases). Maintainers: [`docs/releasing.md`](docs/releasing.md) explains how releases are built, signed and published.

> [!IMPORTANT]
> **Status:** FigCraft is in **alpha**: about 49% of the way to replacing proprietary tools for real work, by our own estimate. Nearly all tracked interface design actions in the menu checklist are wired ([`docs/parity-checklist.md`](docs/parity-checklist.md)) and the core editor works end to end, but FigCraft opens `.fig` files only best effort, has only a first preview of multiplayer (live sessions hosted from the app or a self-hosted server, with per-person undo and share links to edit or to view; no accounts, teams or cloud yet), runs its own WebAssembly plugins, and hasn't been checked against real design documents. The honest, area-by-area picture is in the [roadmap](ROADMAP.md) and [`docs/target-app-parity.md`](docs/target-app-parity.md). Expect rough edges and please file issues, or tell us what broke on [Discord](https://discord.gg/artcraft).

## Privacy and online sharing

FigCraft works offline: it has no account and no telemetry, and it contacts no server until you
share or join a file. Live sessions are a **preview**. Before you share anything that matters,
know what sharing does:

- **Share ▸ Online uploads the file to a server.** By default that is the public server at
  `collab.craftapps.dev`; you can set another one, or your own, in Preferences (and a build can
  ship without it: [`docs/releasing.md`](docs/releasing.md#the-default-server)). The file stays on
  the server after you stop sharing, until you remove it (Share ▸ Advanced ▸ Remove from server…)
  or nobody has opened it for 30 days, when a public server deletes it.
- **It is not end-to-end encrypted.** The connection to the server is encrypted (TLS), but the
  server stores the file unencrypted and whoever runs the server can read it. Don't share
  confidential work through a server you don't trust; run your own
  ([`docs/server.md`](docs/server.md)) or share on the local network instead.
- **The code or link is the access.** There are no accounts: anyone who has an edit code or link
  can open and change the file, anyone with a view code or link can open it. Stop sharing revokes
  them and disconnects everyone else.
- **Share ▸ Local network involves no server**, but it is not encrypted either: your app hosts
  the session and others connect to it directly, so use it on networks you trust.
- **No guarantees.** The public server is offered as is, without backups or uptime promises, and
  files on it can be removed. Keep your own saved copy of anything you share.

The default server's terms are in [`docs/online-sharing-terms.md`](docs/online-sharing-terms.md).

## The Crafting Apps

FigCraft is one of the **Crafting Apps**: free, open-source creative tools from the
[ArtCraft](https://getartcraft.com/) team, each written from scratch in Rust and each able to
stand on its own.

| | App | What it's for | Code | Learn more |
|:-:|---|---|---|---|
| <img src="assets/app-icon/hicolor/64x64/apps/ai.storyteller.figcraft.png" alt="" width="32" height="32"> | **FigCraft** | **Interface design, components and prototypes · you are here** | [GitHub](https://github.com/storytold/figcraft) | [Website](https://getartcraft.com/apps/figcraft) |
| <img src="https://raw.githubusercontent.com/storytold/photocraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.photocraft.png" alt="" width="32" height="32"> | **PhotoCraft** | Image editing | [GitHub](https://github.com/storytold/photocraft) | [Website](https://getartcraft.com/apps/photocraft) |
| <img src="https://raw.githubusercontent.com/storytold/vectorcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.vectorcraft.png" alt="" width="32" height="32"> | **VectorCraft** | Vector illustration | [GitHub](https://github.com/storytold/vectorcraft) | [Website](https://getartcraft.com/apps/vectorcraft) |
| <img src="https://raw.githubusercontent.com/storytold/filmcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.filmcraft.png" alt="" width="32" height="32"> | **FilmCraft** | Video editing, color and sound | [GitHub](https://github.com/storytold/filmcraft) | [Website](https://getartcraft.com/apps/filmcraft) |
| <img src="https://raw.githubusercontent.com/storytold/lightcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.lightcraft.png" alt="" width="32" height="32"> | **LightCraft** | Photo library and raw development | [GitHub](https://github.com/storytold/lightcraft) | [Website](https://getartcraft.com/apps/lightcraft) |
| <img src="https://raw.githubusercontent.com/storytold/pdfcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.pdfcraft.png" alt="" width="32" height="32"> | **PdfCraft** | Reading, organizing and protecting PDFs | [GitHub](https://github.com/storytold/pdfcraft) | [Website](https://getartcraft.com/apps/pdfcraft) |
| <img src="https://raw.githubusercontent.com/storytold/effectcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.effectcraft.png" alt="" width="32" height="32"> | **EffectCraft** | Motion graphics and visual effects | [GitHub](https://github.com/storytold/effectcraft) | [Website](https://getartcraft.com/apps/effectcraft) |
| <img src="https://raw.githubusercontent.com/storytold/designcraft/main/assets/app-icon/hicolor/64x64/apps/ai.storyteller.designcraft.png" alt="" width="32" height="32"> | **DesignCraft** | Page layout and publishing | [GitHub](https://github.com/storytold/designcraft) | [Website](https://getartcraft.com/apps/designcraft) |

And [**ArtCraft**](https://getartcraft.com/) itself, our AI image and video studio for artists who want real control.

The Crafting Apps share the same conventions: clean-room and pure Rust, native on macOS, Windows, Linux and FreeBSD, in the browser via WebAssembly, and fully drivable by agents.

<br>

<p align="center">
  <a href="https://discord.gg/artcraft"><img alt="Join the ArtCraft community on Discord" src="https://img.shields.io/badge/Join%20us%20on%20Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="40"></a>
</p>

<h3 align="center">Come make things with us</h3>

<p align="center">
  Our Discord is where people who make things hang out: designers sketching their first app,
  developers who just want a button to look right, illustrators, filmmakers, type nerds, and
  plenty of people still working out what they like to make. Share what you're building, ask for
  help, tell us what's broken or what you wish FigCraft could do. You're welcome here.
</p>

<p align="center">
  <a href="https://discord.gg/artcraft"><b>discord.gg/artcraft</b></a> ·
  <a href="https://getartcraft.com/">getartcraft.com</a> ·
  <a href="https://getartcraft.com/apps">The Crafting Apps</a> ·
  <a href="https://getartcraft.com/apps/figcraft">FigCraft</a>
</p>

---

## License and credits

FigCraft is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the FigCraft contributors. Required notices are in [NOTICE](NOTICE).

Every icon in FigCraft is drawn in code or made for this project, and bundled fonts, images and other
assets keep their own open licenses; each one is listed with its author, source and license in
[ATTRIBUTION.md](ATTRIBUTION.md).

The ArtCraft name, wordmark and logos in [`docs/brand/`](docs/brand/) are trademarks of the
ArtCraft Team and are not covered by this license. They may be used only unmodified, and only as
part of this repository and FigCraft, under [`docs/brand/LICENSE-brand.txt`](docs/brand/LICENSE-brand.txt).
Forks and modified versions must remove them.

<p align="center">
  <a href="https://getartcraft.com/"><img alt="ArtCraft" src="https://raw.githubusercontent.com/storytold/vectorcraft/main/docs/brand/artcraft-mark.png" width="28"></a><br>
  <sub>Made by the <a href="https://getartcraft.com/">ArtCraft</a> team and community.</sub>
</p>

## Star history

[![Star History Chart](https://api.star-history.com/svg?repos=storytold/figcraft&type=Date&legend=top-left)](https://www.star-history.com/?repos=storytold%2Ffigcraft&type=date&legend=top-left)
