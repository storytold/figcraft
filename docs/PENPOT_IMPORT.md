# Penpot import and export

FigCraft reads and writes Penpot 2.x ZIP archives (`penpot/export-files`, version 1)
in Rust. Import and export run offline on desktop, CLI, MCP and WASM.

## Open and save

- **File → Open…** opens a `.penpot` as a separate unsaved document. Dropping it
  onto the canvas does the same. The original archive is not overwritten.
- **File → Import from file…** adds its pages to the current document.
- **Save / Save as…** writes native `.figcraft` files.
- **Export** with format `penpot` writes an archive for a Penpot team.

Engine commands use the same importer and exporter:

```json
{"command":"file.open","params":{"path":"example.penpot"}}
{"command":"file.import","params":{"path":"example.penpot"}}
{"command":"file.export","params":{"format":"penpot","path":"out.penpot"}}
```

Without a path, `file.export` returns base64 `data`. Unsupported conversions are
reported in `warnings`; the CLI prints them as notes.

```sh
figcraft-cli info example.penpot --json
figcraft-cli convert example.penpot editable.figcraft
figcraft-cli export editable.figcraft -o out.penpot --format penpot
```

## Supported content

| Area | Support |
|---|---|
| Structure | Pages, boards, groups, masks, nested frames, editable shapes and vectors |
| Appearance | Solid, gradient and image fills; strokes, shadows, blur, opacity, blending and transforms |
| Text | Paragraphs, styled runs, alignment, sizing, decorations and original font references |
| Layout | Constraints, guides, display grids, flex/grid layout, reverse flow, margins, signed padding, align-self, stretch, wrapped-line spacing and percentage tracks |
| Components | Local main components, linked copies, overrides and variant sets; unresolved external copies remain independent with a warning |
| Tokens | Supported scalar tokens, aliases, theme modes, expressions and matching property bindings; literal compound typography/shadow tokens can become native styles |
| Prototypes | Flows, supported pointer/delay triggers, navigation, overlays, toggle overlay, back and links, with supported transitions |

Penpot source ids and selected source properties remain in layer metadata.
Fingerprints let export reuse unchanged text, layout, strokes, interactions and
tokens instead of regenerating them. Edited supported values are generated from
FigCraft's model. New layers receive deterministic UUIDs.

## Fonts

Embedded TTF, OTF, TTC, OTC, WOFF1 and WOFF2 fonts are imported without installation.
WOFF containers are converted to SFNT in Rust. Fonts are selected per document,
including measurement and rendering; another tab's same-named family cannot
replace them. Parsed faces use a shared bounded LRU cache.

Native `.figcraft` saves preserve the binaries, licences and ownership in version
history and branches. Imports enforce limits of 128 fonts, 32 MiB per font and
128 MiB of font data. Excess or invalid fonts are skipped and reported instead
of preventing the document from being saved. Active-document fonts take priority
over historical fonts when an archive exceeds the combined budget.

Provider references never trigger downloads. `file.fontReport` reports availability
and distinguishes Google provider references from custom library references whose
original binary cannot be verified. Missing fonts use local fallback while their
original family names remain editable.

Penpot export writes font references and warns that embedded font binaries are
omitted. Install the referenced fonts in the destination Penpot team; save as
`.figcraft` to retain the binaries.

## Limits

- Penpot 1.x binary exports are unsupported; re-export from Penpot 2.x.
- Malformed archives, oversized members and excessive nesting/layer counts are
  rejected or bounded. Unsupported features produce import/export warnings.
- Raw SVG uses FigCraft's SVG importer where possible and retains source metadata.
  Complex masks and unsupported geometry may remain placeholders with warnings.
- Penpot export approximates unsupported paints/effects, custom dashes and
  transitions; unsupported actions and triggers are reported. Slices are omitted.
- Regenerated text has no Penpot `positionData`; Penpot lays it out again.
- An imported `tokens.json` is preserved when token changes cannot be regenerated;
  the exporter reports that limitation.
- `.fig` keeps reverse flow, stretch, wrapped-line spacing and percentage tracks
  as `figcraft`/`layout` plugin data. Native track sizes remain authoritative;
  other readers ignore the extension.

## Verification

`cargo xtask ci` passes on Windows with Rust 1.95, including WASM, architecture,
asset attribution and example plugins. A synthetic document was exported, opened
in the desktop app, saved natively, reopened and visually checked. CLI conversion
and export, plus an external sample's offline round trip, were also checked.
This does not establish native macOS/Linux or live Penpot server compatibility.

```sh
cargo xtask ci
cargo test -p figcraft-format penpot
cargo test -p figcraft-engine penpot
```

Optional local samples are not distributed or required:

```sh
FIGCRAFT_PENPOT_SAMPLE=/path/to/example.penpot cargo test -p figcraft-format penpot
FIGCRAFT_PENPOT_SAMPLES=/path/to/folder cargo test -p figcraft-engine penpot_export_tests
```
