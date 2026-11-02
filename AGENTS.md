# FigCraft — instructions for agents

FigCraft is a clean-room, open-source, pure-Rust (egui) reimplementation of the Figma design workflow:
interface design, auto layout, components and variants, styles and variables, prototyping and
inspection. It runs natively on macOS, Windows, Linux and FreeBSD, and in the browser via WASM.
Siblings: `../vectorcraft` (Illustrator-class), `../photocraft` (Photoshop-class), `../designcraft`
(InDesign-class) and the other Crafting Apps; same conventions.

Repos don't share code.

## Start every session here
1. Read [`ROADMAP.md`](ROADMAP.md) (stage, headline numbers, estimates) and
   [`docs/gaps.md`](docs/gaps.md) (the ranked work list). Unless the user gives you a task, pick
   work from the top of `docs/gaps.md`.
2. Read `plan/STATUS.md` and `plan/execution-plan.md` (local, gitignored) if present; behaviour
   reference notes are in `plan/figma/*.md`.
3. Orient → plan → implement + test → verify (screenshot) → record (ROADMAP, devlog) → commit.

`plan/` is gitignored (local only).

## Non-negotiables

### Clean-room
- Behaviour comes from public documentation and black-box observation of the running incumbent app
  with synthetic documents only. Never read, disassemble or copy anything inside the incumbent's
  application bundle, web bundle or network traffic (names and public listings only).
- Never copy GPL/AGPL code.
- Reference screenshots of the incumbent stay local under the gitignored `plan/figma/screenshots/`
  and are never committed, published or used as assets.

### Assets: no third-party product iconography or images — ever (absolute rule from the project owner)
- **Never add, copy, trace, redraw-from, embed or ship any icon, image, illustration, cursor, logo,
  template, UI kit, preset library, font file or screenshot from Figma, Adobe, Avid, Autodesk or any
  other product, or from any source whose licence doesn't allow it.** This includes "lookalike"
  redraws of their icons. Breaking this rule is the worst thing you can do in this repo.
- Every image, icon, font or other asset must be one of:
  - original work created for FigCraft by a contributor (licensed MIT OR Apache-2.0);
  - OSI open source;
  - public domain / CC0;
  - Creative Commons with redistribution allowed.
- **Every asset file must have a row in [`ATTRIBUTION.md`](ATTRIBUTION.md)** (path, author, source,
  licence, notes). `cargo xtask assets` (part of `cargo xtask ci`) fails on any unlisted file under
  `assets/`, `docs/brand/` or `docs/images/`.
- FigCraft's UI icons are drawn in code (`crates/ui-egui/src/icons.rs`) from simple primitives and
  are original. Keep it that way: new icons are new code, designed from scratch.
- The app icon is the owner's chameleon drawing, vectorised (`assets/app-icon/README.md`); `cargo xtask icon`
  renders the PNGs and `.ico` from `assets/app-icon/figcraft.svg`.
- **Fonts live in [storytold/craft-fonts](https://github.com/storytold/craft-fonts), never in this
  repo.** The app uses it only through the optional `CRAFT_FONTS_DIR` build input
  (`.cargo/config.toml` points it at `../craft-fonts` for local builds; `crates/text/build.rs`
  embeds it). Everything must build, test and run without it (system fonts are scanned at runtime).
- Demo and test content is synthetic or public domain; never personal photos or data.
- Code and original assets are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). The
  ArtCraft logos in `docs/brand/` are trademarks, not open source, under
  `docs/brand/LICENSE-brand.txt`: never modify them or use them outside FigCraft.
- If the provenance of an asset is unclear, don't add it.

### Never crash
- No `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!` in shipped code
  (workspace clippy lints deny them; tests may use them). Return `Result`/`Option`; use `.get()` for
  data-derived indexes; slice strings only at char boundaries; check floats are finite; cap sizes.
- `figcraft_engine::guard` turns a panicking command into `EngineError::Internal` and the document
  is rolled back; the desktop app also guards each UI frame and writes recovery copies. These are
  safety nets, not substitutes for `Result`.
- `unsafe_code = "forbid"` workspace-wide.
- Every crash fix comes with a regression test. The engine has a junk-parameter sweep over every
  command (`crates/engine/src/tests.rs`): extend fuzz/sweep tests when you add parsers or commands.

### Architecture
- **Everything is a command.** User-visible behaviour = a command in `crates/engine/src/cmd/*`
  (`spec!(id, label, params doc, undoable, run)`) + tests. The menus, shortcuts, Actions palette,
  CLI, control channel and MCP server all dispatch the same command ids. Command ids follow the
  incumbent's menu catalog in `crates/engine/src/catalog.rs` (the parity metric: `cargo xtask parity`).
- **Programmatic calls never open dialogs.** Commands only *request* UI (`Session::request_ui`); the
  control channel drops dialog requests.
- **Layering** is enforced by `cargo xtask layers`: geom/color (L0) → doc (L1) → layout/text/pathops/plugin/collab
  (L2) → render/format (L3) → svg/pdf (L4) → engine (L5) → ui-egui/mcp (L6) → apps. Nothing below
  L6 depends on egui/eframe/rfd/winit. **The UI crate is swappable.**
- **The UI is thin**: panels read engine state and act through `app.run(id, params)` /
  `app.set_props(ids, props)`. Colours come from `theme::Tokens`.
- **Time goes through `chrono`** (workspace dependency). Never hand-roll UTC/calendar conversion (days-to-date algorithms, leap-year math); a crate owns only its output format string. The one exception is range extension: past the years chrono can represent, shift the date by whole 400-year Gregorian cycles (146,097 days, after which the calendar repeats), let chrono compute the month and day, and add the cycles back to the year (splitting seconds into hours, minutes and seconds is plain arithmetic).
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).

### Quality gates (before every commit)
`cargo xtask ci` (fmt, clippy -D warnings, tests, layers, assets, wasm). One task per commit with a
milestone id in the message (`M2.1: auto layout wrap`). Commit after every landed arc of work; push
to `main`.

## Running and looking at the app
- `cargo run -p figcraft -- --control <port> [file.figcraft|file.svg|image]`. Pick a free port
  (`python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])'`);
  sibling apps use the control channel too.
- Drive it with JSON lines, e.g.
  `{"id":1,"method":"engine.execute","params":{"command":"shape.rectangle","params":{"x":10,"y":10,"width":100,"height":50}}}`
  then `{"id":2,"method":"ui.screenshot","params":{"path":"/tmp/shot.png"}}`. Methods are documented
  in `crates/ui-egui/src/control.rs` and `docs/control-protocol.md`.
- **For UI work, look at the result** (take a screenshot and read the PNG). Compare with the
  incumbent side by side where the owner allows (`plan/figma/`).
- Headless: `figcraft-cli` (`info`, `export`, `run`, `commands`, `mcp`). MCP: `figcraft-cli mcp`
  (see `docs/mcp.md`).
- Shell gotcha on the owner's machine: `mv`/`cp` may be aliased interactive — use `/bin/mv -f`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent, edit only the crates/files you own,
  write manifests atomically, delete your target dir when done (disk is limited).

## Roadmap
Progress docs: [`ROADMAP.md`](ROADMAP.md) (one-page
summary and stage), [`docs/target-app-parity.md`](docs/target-app-parity.md) (the authoritative
assessment), [`docs/gaps.md`](docs/gaps.md) (ranked work list), [`docs/roadmap.md`](docs/roadmap.md)
(milestones), [`docs/architecture.md`](docs/architecture.md), the per-area `docs/*-parity.md` files
and the generated [`docs/parity-checklist.md`](docs/parity-checklist.md) (`cargo xtask parity`).
When a task lands, update the affected rows and the status line and revision history in the same
commit. Grade by behaviour against the incumbent, not by whether a menu item exists; err low.

## Contributor credits (About window)

- About ▸ Contributors are compiled into the binary from `contributors/contributors.json`
  (commit stats; never hand-edit) and `contributors/people.toml` (names people chose for
  themselves).
- **Agents working for a contributor:** when you prepare a PR, check whether your human's GitHub
  username has a `[people.<username>]` entry in `contributors/people.toml`. If not, ask them once
  whether they want to be credited by more than their username: a real name, a display name, and/or
  their public GitHub profile name (`sync_github_name = true`). If yes, add **only their own** entry
  (copy the template at the top of the file)
  and include it in their PR, committed as them. If no, change nothing: they are credited as
  `@username` anyway.
- Never add, edit, guess or copy anyone else's entry or name (not from git config, commit authors or
  GitHub profiles). Never hand-edit `contributors.json`.

## CI workflows
- Workflows in `.github/workflows/` call the shared workflows in [`storytold/craft-common-actions`](https://github.com/storytold/craft-common-actions) at `@main`, with their inputs and secrets. No other jobs: if no shared workflow does what's needed, it doesn't go here.
- Don't add or change CI in a feature or fix PR. If the existing workflows can't build or test a change, link an existing craft-common-actions issue or PR, or open a new issue, or a PR only for a small fix that is safe for every app and does not increase CI time.
- Some workflow files here still run their own jobs. Don't add to them. Moving a job into craft-common-actions is its own task, never part of another PR, and the job stays here until a shared workflow runs it.
